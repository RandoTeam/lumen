package dev.snapdragonllm;

import android.app.Activity;
import android.app.ActivityManager;
import android.content.Context;
import android.content.SharedPreferences;
import android.content.res.Configuration;
import android.graphics.Color;
import android.graphics.Typeface;
import android.net.Uri;
import android.os.Build;
import android.os.Bundle;
import android.os.StatFs;
import android.util.Log;
import android.util.TypedValue;
import android.view.Gravity;
import android.view.View;
import android.view.WindowInsets;
import android.view.WindowInsetsController;
import android.view.WindowManager;
import android.webkit.JavascriptInterface;
import android.webkit.WebResourceRequest;
import android.webkit.WebResourceResponse;
import android.webkit.WebSettings;
import android.webkit.WebView;
import android.webkit.WebViewClient;
import android.widget.FrameLayout;
import android.widget.ImageView;
import android.widget.LinearLayout;
import android.widget.TextView;

import java.io.BufferedInputStream;
import java.io.BufferedOutputStream;
import java.io.File;
import java.io.FileOutputStream;
import java.io.IOException;
import java.io.InputStream;
import java.io.OutputStream;
import java.net.HttpURLConnection;
import java.net.InetSocketAddress;
import java.net.Proxy;
import java.net.URL;
import java.net.URLEncoder;
import java.nio.charset.StandardCharsets;
import java.util.Locale;

import org.json.JSONArray;
import org.json.JSONObject;

/**
 * Minimal host for the Preact UI. All model work happens in Rust behind JNI;
 * this class only owns the Activity, the system WebView and the asset loader.
 */
public class MainActivity extends Activity {
    static {
        System.loadLibrary("snapdragonllmweb");
    }

    private static final String TAG = "SLLM";
    static final String DEFAULT_SMOKE_MODEL =
            "/storage/emulated/0/Android/data/dev.snapdragonllm/files/models/tiny/"
                    + "Qwen2.5-0.5B-Instruct-Q4_K_M.gguf";

    private WebView web;
    private FrameLayout rootView;
    private View splashView;
    private String themePref = "system";

    /** Set before tearing the WebView down; posted UI callbacks check it. */
    private volatile boolean destroyed;
    /** Set by `am start ... --es smoke 1`; runs once after the page loads. */
    private boolean smokeOnLoad;
    private volatile Thread smokeThread;
    private volatile Thread hfThread;
    private volatile HttpURLConnection hfConnection;
    private volatile boolean hfCancel;
    private String debugProxyHost;
    private int debugProxyPort;

    public static native String nativeVersion();

    public static native String nativeModels(String dir, int availMb);

    /** llama.cpp version from the linked native library. */
    public static native String nativeLlamaVersion();

    /**
     * Load a GGUF, tokenize a short prompt, prefill and decode up to 4 tokens.
     * Blocks the calling thread; returns a JSON report.
     */
    public static native String nativeDeviceSmoke(String modelPath);

    public static native void nativeLlamaUnload();

    /** Name of the attached engine: "llama" when real inference is linked. */
    public static native String nativeEngine();

    /** Start a generation; returns a JSON ack immediately. */
    public static native String nativeStart(String requestJson, String modelsRoot);

    /** One queued generation event as JSON, or null when the queue is empty. */
    public static native String nativePoll();

    /** Load a model without generating; model events arrive through poll(). */
    public static native String nativePrepare(String modelId, String modelPath, String modelsRoot);

    public static native void nativeCancel();

    public static native String nativeStats();

    public static native void nativeUnload();

    /** Android onTrimMemory level: "moderate", "running_low" or "complete". */
    public static native void nativeTrim(String level);

    /** Models live in Android/data/dev.snapdragonllm/files/models. */
    private String modelsDir() {
        File base = getExternalFilesDir(null);
        if (base == null) {
            base = getFilesDir();
        }
        return new File(base, "models").getAbsolutePath();
    }

    @Override
    protected void onCreate(Bundle state) {
        super.onCreate(state);
        getWindow().setSoftInputMode(WindowManager.LayoutParams.SOFT_INPUT_ADJUST_RESIZE);
        themePref = readThemePref();
        int background = themeBackground(themePref);

        if (Build.VERSION.SDK_INT >= 30) {
            getWindow().setDecorFitsSystemWindows(false);
        } else {
            getWindow().getDecorView().setSystemUiVisibility(
                    View.SYSTEM_UI_FLAG_LAYOUT_STABLE
                            | View.SYSTEM_UI_FLAG_LAYOUT_FULLSCREEN
                            | View.SYSTEM_UI_FLAG_LAYOUT_HIDE_NAVIGATION);
        }
        getWindow().setStatusBarColor(Color.TRANSPARENT);
        getWindow().setNavigationBarColor(Color.TRANSPARENT);

        // Opt-in WebView debugging for on-device smoke tests:
        //   adb shell am start -n dev.snapdragonllm/.MainActivity --ez sllm.debug true
        // Normal launches keep debugging off.
        if (getIntent().getBooleanExtra("sllm.debug", false)) {
            WebView.setWebContentsDebuggingEnabled(true);
        }
        // Debug-only proxy override for host-side network tests:
        //   adb shell am start ... --es sllm.proxy 127.0.0.1:8888
        String proxy = getIntent().getStringExtra("sllm.proxy");
        if (proxy != null && proxy.contains(":")) {
            String host = proxy.substring(0, proxy.lastIndexOf(':'));
            String port = proxy.substring(proxy.lastIndexOf(':') + 1);
            System.setProperty("http.proxyHost", host);
            System.setProperty("http.proxyPort", port);
            System.setProperty("https.proxyHost", host);
            System.setProperty("https.proxyPort", port);
            debugProxyHost = host;
            debugProxyPort = Integer.parseInt(port);
            Log.i(TAG, "debug proxy " + host + ":" + port);
        }

        // adb shell am start -n dev.snapdragonllm/.MainActivity --es smoke 1
        String smokeExtra = getIntent().getStringExtra("smoke");
        smokeOnLoad = "1".equals(smokeExtra) || getIntent().getBooleanExtra("smoke", false);

        web = new WebView(this);
        web.setBackgroundColor(background);

        WebSettings settings = web.getSettings();
        settings.setJavaScriptEnabled(true);
        settings.setDomStorageEnabled(true);
        settings.setAllowFileAccess(true);
        settings.setAllowContentAccess(true);
        settings.setMediaPlaybackRequiresUserGesture(false);
        settings.setSupportZoom(false);

        web.addJavascriptInterface(new Bridge(), "Sllm");
        web.setFocusable(true);
        web.setFocusableInTouchMode(true);
        web.requestFocus();
        web.setWebViewClient(new WebViewClient() {
            @Override
            public WebResourceResponse shouldInterceptRequest(WebView view, WebResourceRequest request) {
                Uri uri = request.getUrl();
                if (uri != null && "app.local".equals(uri.getHost())) {
                    String path = uri.getPath();
                    if (path == null || path.isEmpty() || "/".equals(path)) {
                        path = "/index.html";
                    }
                    try {
                        InputStream stream = getAssets().open(path.substring(1));
                        return new WebResourceResponse(mimeOf(path), "utf-8", stream);
                    } catch (IOException ignored) {
                        return new WebResourceResponse("text/plain", "utf-8", null);
                    }
                }
                return null;
            }

            @Override
            public void onPageFinished(WebView view, String url) {
                super.onPageFinished(view, url);
                hideSplash();
                if (smokeOnLoad) {
                    smokeOnLoad = false;
                    startLlamaSmoke(DEFAULT_SMOKE_MODEL);
                }
            }
        });

        rootView = new FrameLayout(this);
        rootView.setBackgroundColor(background);
        rootView.addView(web, new FrameLayout.LayoutParams(-1, -1));
        splashView = buildSplash(background);
        rootView.addView(splashView, new FrameLayout.LayoutParams(-1, -1));
        setContentView(rootView);
        applyInsets(rootView);
        applyWindowTheme(themePref);
        web.loadUrl("https://app.local/index.html");
    }

    private int dp(int value) {
        return Math.round(TypedValue.applyDimension(
                TypedValue.COMPLEX_UNIT_DIP, value, getResources().getDisplayMetrics()));
    }

    /** Native launch screen: covers WebView cold start with the Lumen mark. */
    private View buildSplash(int background) {
        LinearLayout box = new LinearLayout(this);
        box.setOrientation(LinearLayout.VERTICAL);
        box.setGravity(Gravity.CENTER);
        box.setBackgroundColor(background);

        ImageView icon = new ImageView(this);
        int iconSize = dp(76);
        icon.setLayoutParams(new LinearLayout.LayoutParams(iconSize, iconSize));
        int iconId = getResources().getIdentifier("ic_launcher", "mipmap", getPackageName());
        if (iconId != 0) {
            icon.setImageResource(iconId);
        }
        box.addView(icon);

        boolean light = "light".equals(resolveTheme(themePref));
        int ink = light ? Color.rgb(23, 24, 19) : Color.rgb(237, 234, 226);

        TextView name = new TextView(this);
        name.setText("LUMEN");
        name.setTextSize(TypedValue.COMPLEX_UNIT_SP, 13);
        name.setLetterSpacing(0.36f);
        name.setTypeface(Typeface.create("sans-serif-medium", Typeface.NORMAL));
        name.setTextColor(ink);
        name.setPadding(0, dp(24), 0, 0);
        box.addView(name);

        TextView tagline = new TextView(this);
        tagline.setText("local intelligence");
        tagline.setTextSize(TypedValue.COMPLEX_UNIT_SP, 12);
        tagline.setTextColor(ink);
        tagline.setAlpha(0.55f);
        tagline.setPadding(0, dp(8), 0, 0);
        box.addView(tagline);

        return box;
    }

    private void hideSplash() {
        View splash = splashView;
        if (splash == null) {
            return;
        }
        splashView = null;
        splash.animate()
                .alpha(0f)
                .setDuration(260)
                .withEndAction(() -> {
                    if (rootView != null) {
                        rootView.removeView(splash);
                    }
                })
                .start();
    }

    private static String mimeOf(String path) {
        String lower = path.toLowerCase(Locale.ROOT);
        if (lower.endsWith(".html")) return "text/html";
        if (lower.endsWith(".js")) return "application/javascript";
        if (lower.endsWith(".css")) return "text/css";
        if (lower.endsWith(".json")) return "application/json";
        if (lower.endsWith(".svg")) return "image/svg+xml";
        if (lower.endsWith(".png")) return "image/png";
        if (lower.endsWith(".jpg") || lower.endsWith(".jpeg")) return "image/jpeg";
        if (lower.endsWith(".woff2")) return "font/woff2";
        return "application/octet-stream";
    }

    private String readThemePref() {
        SharedPreferences prefs = getSharedPreferences("sllm", MODE_PRIVATE);
        return prefs.getString("theme", "system");
    }

    private String resolveTheme(String pref) {
        if (!"system".equals(pref)) {
            return pref;
        }
        boolean night = (getResources().getConfiguration().uiMode
                & Configuration.UI_MODE_NIGHT_MASK) == Configuration.UI_MODE_NIGHT_YES;
        return night ? "dark" : "light";
    }

    private int themeBackground(String pref) {
        switch (resolveTheme(pref)) {
            case "dark":
                return Color.rgb(27, 28, 26);
            case "night":
                return Color.rgb(32, 28, 23);
            case "amoled":
                return Color.rgb(0, 0, 0);
            default:
                return Color.rgb(245, 242, 234);
        }
    }

    private void applyBarIcons(String resolved) {
        boolean light = "light".equals(resolved);
        if (Build.VERSION.SDK_INT >= 30) {
            WindowInsetsController controller = getWindow().getInsetsController();
            if (controller != null) {
                int mask = WindowInsetsController.APPEARANCE_LIGHT_STATUS_BARS
                        | WindowInsetsController.APPEARANCE_LIGHT_NAVIGATION_BARS;
                controller.setSystemBarsAppearance(light ? mask : 0, mask);
            }
        } else {
            int flags = View.SYSTEM_UI_FLAG_LAYOUT_STABLE
                    | View.SYSTEM_UI_FLAG_LAYOUT_FULLSCREEN
                    | View.SYSTEM_UI_FLAG_LAYOUT_HIDE_NAVIGATION;
            if (light) {
                flags |= View.SYSTEM_UI_FLAG_LIGHT_STATUS_BAR
                        | View.SYSTEM_UI_FLAG_LIGHT_NAVIGATION_BAR;
            }
            getWindow().getDecorView().setSystemUiVisibility(flags);
        }
    }

    private void applyWindowTheme(String pref) {
        themePref = pref;
        String resolved = resolveTheme(pref);
        int background = themeBackground(pref);
        if (rootView != null) {
            rootView.setBackgroundColor(background);
        }
        if (web != null) {
            web.setBackgroundColor(background);
        }
        applyBarIcons(resolved);
    }

    @Override
    public void onConfigurationChanged(Configuration newConfig) {
        super.onConfigurationChanged(newConfig);
        if ("system".equals(themePref)) {
            applyWindowTheme("system");
            runOnUiThread(() -> {
                if (web != null) {
                    web.evaluateJavascript(
                            "window.sllmSystemTheme&&window.sllmSystemTheme('"
                                    + resolveTheme("system") + "')", null);
                }
            });
        }
    }

    private void applyInsets(View root) {
        root.setOnApplyWindowInsetsListener((view, insets) -> {
            int left;
            int top;
            int right;
            int bottom;
            if (Build.VERSION.SDK_INT >= 30) {
                android.graphics.Insets bars = insets.getInsets(WindowInsets.Type.systemBars());
                left = bars.left;
                top = bars.top;
                right = bars.right;
                bottom = bars.bottom;
                if (insets.isVisible(WindowInsets.Type.ime())) {
                    android.graphics.Insets ime = insets.getInsets(WindowInsets.Type.ime());
                    bottom = Math.max(bottom, ime.bottom);
                }
            } else {
                left = insets.getSystemWindowInsetLeft();
                top = insets.getSystemWindowInsetTop();
                right = insets.getSystemWindowInsetRight();
                bottom = insets.getSystemWindowInsetBottom();
            }
            view.setPadding(left, top, right, bottom);
            return insets;
        });
        root.requestApplyInsets();
    }

    /** Run the smoke test on a worker thread and report through window.sllmSmoke. */
    private void startLlamaSmoke(String modelPath) {
        Thread running = smokeThread;
        if (running != null && running.isAlive()) {
            Log.w(TAG, "llama smoke is already running");
            return;
        }
        Thread thread = new Thread(() -> {
            String json = nativeDeviceSmoke(modelPath);
            Log.i(TAG, "llama smoke result: " + json);
            runOnUiThread(() -> {
                if (destroyed || web == null) {
                    return;
                }
                web.evaluateJavascript("window.sllmSmoke&&window.sllmSmoke(" + json + ")", null);
            });
        }, "sllm-smoke");
        smokeThread = thread;
        thread.start();
    }

    @Override
    protected void onDestroy() {
        destroyed = true;
        // Freeing a model can take a moment; never block the UI thread.
        new Thread(() -> {
            nativeCancel();
            nativeLlamaUnload();
            nativeUnload();
        }, "sllm-unload").start();
        if (web != null) {
            web.destroy();
            web = null;
        }
        super.onDestroy();
    }

    @Override
    public void onTrimMemory(int level) {
        super.onTrimMemory(level);
        if (level >= TRIM_MEMORY_COMPLETE) {
            nativeTrim("complete");
        } else if (level >= TRIM_MEMORY_RUNNING_LOW) {
            nativeTrim("running_low");
        } else if (level >= TRIM_MEMORY_RUNNING_MODERATE) {
            nativeTrim("moderate");
        }
    }

    private void postJs(final String code) {
        runOnUiThread(() -> {
            if (!destroyed && web != null) {
                web.evaluateJavascript(code, null);
            }
        });
    }

    private static String enc(String value) {
        try {
            return URLEncoder.encode(value, StandardCharsets.UTF_8.name());
        } catch (Exception ignored) {
            return value;
        }
    }

    private String hfToken() {
        return getSharedPreferences("sllm", MODE_PRIVATE).getString("hf.token", "");
    }

    private HttpURLConnection hfOpen(String url, String accept, boolean range, long offset)
            throws IOException {
        URL target = new URL(url);
        HttpURLConnection connection;
        if (debugProxyHost != null) {
            Proxy proxy = new Proxy(
                    Proxy.Type.HTTP, new InetSocketAddress(debugProxyHost, debugProxyPort));
            connection = (HttpURLConnection) target.openConnection(proxy);
        } else {
            connection = (HttpURLConnection) target.openConnection();
        }
        connection.setConnectTimeout(20000);
        connection.setReadTimeout(60000);
        connection.setInstanceFollowRedirects(true);
        connection.setRequestProperty("User-Agent", "Lumen/0.2 (+https://github.com/RandoTeam/lumen)");
        connection.setRequestProperty("Accept", accept);
        String token = hfToken();
        if (!token.isEmpty()) {
            connection.setRequestProperty("Authorization", "Bearer " + token);
        }
        if (range && offset > 0) {
            connection.setRequestProperty("Range", "bytes=" + offset + "-");
        }
        return connection;
    }

    private JSONArray hfRequestModels(
            String query, String author, String sort, int limit) throws Exception {
        StringBuilder url = new StringBuilder("https://huggingface.co/api/models?limit=")
                .append(limit)
                .append("&full=false")
                .append("&filter=gguf");
        if (query != null && !query.trim().isEmpty()) {
            url.append("&search=").append(enc(query.trim()));
        }
        if (author != null && !author.trim().isEmpty()) {
            url.append("&author=").append(enc(author.trim()));
        }
        if ("downloads".equals(sort)) {
            url.append("&sort=downloads&direction=-1");
        } else if ("likes".equals(sort)) {
            url.append("&sort=likes&direction=-1");
        } else if ("updated".equals(sort)) {
            url.append("&sort=lastModified&direction=-1");
        } else if ("created".equals(sort)) {
            url.append("&sort=createdAt&direction=-1");
        }

        HttpURLConnection connection = hfOpen(url.toString(), "application/json", false, 0);
        try {
            int code = connection.getResponseCode();
            InputStream stream = code >= 200 && code < 300
                    ? connection.getInputStream()
                    : connection.getErrorStream();
            String body = readAll(stream);
            if (code < 200 || code >= 300) {
                throw new IOException("Hugging Face returned HTTP " + code + ": " + body);
            }
            return new JSONArray(body);
        } finally {
            connection.disconnect();
        }
    }

    private static String readAll(InputStream stream) throws IOException {
        if (stream == null) {
            return "";
        }
        try (BufferedInputStream input = new BufferedInputStream(stream)) {
            byte[] buffer = new byte[16384];
            StringBuilder out = new StringBuilder();
            int read;
            while ((read = input.read(buffer)) >= 0) {
                out.append(new String(buffer, 0, read, StandardCharsets.UTF_8));
            }
            return out.toString();
        }
    }

    private static boolean isGguf(JSONObject model) {
        JSONArray tags = model.optJSONArray("tags");
        if (tags != null) {
            for (int i = 0; i < tags.length(); i++) {
                if ("gguf".equalsIgnoreCase(tags.optString(i))) return true;
            }
        }
        JSONArray siblings = model.optJSONArray("siblings");
        if (siblings != null) {
            for (int i = 0; i < siblings.length(); i++) {
                JSONObject sibling = siblings.optJSONObject(i);
                if (sibling != null
                        && sibling.optString("rfilename").toLowerCase(Locale.ROOT).endsWith(".gguf")) {
                    return true;
                }
            }
        }
        return false;
    }

    private static String fileKind(String name) {
        String lower = name.toLowerCase(Locale.ROOT);
        if (lower.contains("mmproj") || lower.contains("projector") || lower.contains("vision")) {
            return "mmproj";
        }
        if (lower.contains("lora") || lower.contains("adapter")) {
            return "lora";
        }
        if (lower.endsWith(".gguf")) {
            return "model";
        }
        return "other";
    }

    private void startHfSearch(String requestJson) {
        final JSONObject request;
        try {
            request = new JSONObject(requestJson);
        } catch (Exception error) {
            postJs("window.sllmHfResults({ok:false,error:" + JSONObject.quote("bad request") + "})");
            return;
        }
        cancelHf();
        hfCancel = false;
        hfThread = new Thread(() -> {
            try {
                String query = request.optString("query", "");
                String author = request.optString("author", "");
                String sort = request.optString("sort", "relevance");
                JSONArray models = hfRequestModels(query, author, sort, 30);
                if (models.length() == 0 && !author.trim().isEmpty()) {
                    // HF's author filter is case-sensitive. Resolve the canonical
                    // spelling once, then repeat the exact query.
                    JSONArray probe = hfRequestModels(query, "", sort, 100);
                    String canonical = null;
                    for (int i = 0; i < probe.length(); i++) {
                        JSONObject model = probe.optJSONObject(i);
                        if (model != null
                                && model.optString("author").equalsIgnoreCase(author.trim())) {
                            canonical = model.optString("author");
                            break;
                        }
                    }
                    if (canonical != null) {
                        models = hfRequestModels(query, canonical, sort, 30);
                    }
                }

                JSONArray out = new JSONArray();
                for (int i = 0; i < models.length() && !hfCancel; i++) {
                    JSONObject model = models.optJSONObject(i);
                    if (model == null) continue;
                    JSONObject item = new JSONObject();
                    item.put("id", model.optString("id"));
                    item.put("author", model.optString("author"));
                    item.put("downloads", model.optLong("downloads"));
                    item.put("likes", model.optLong("likes"));
                    item.put("lastModified", model.optString("lastModified"));
                    item.put("createdAt", model.optString("createdAt"));
                    item.put("gated", model.optBoolean("gated"));
                    item.put("pipelineTag", model.optString("pipeline_tag"));
                    item.put("tags", model.optJSONArray("tags"));
                    item.put("gguf", isGguf(model));
                    out.put(item);
                }
                JSONObject response = new JSONObject();
                response.put("ok", true);
                response.put("query", query);
                response.put("author", author);
                response.put("sort", sort);
                response.put("models", out);
                postJs("window.sllmHfResults(" + response + ")");
            } catch (Exception error) {
                postJs("window.sllmHfResults({ok:false,error:"
                        + JSONObject.quote(String.valueOf(error.getMessage())) + "})");
            }
        }, "sllm-hf-search");
        hfThread.start();
    }

    private void startHfFiles(String repo) {
        cancelHf();
        hfCancel = false;
        hfThread = new Thread(() -> {
            try {
                String url = "https://huggingface.co/api/models/"
                        + repo
                        + "?blobs=true";
                HttpURLConnection connection = hfOpen(url, "application/json", false, 0);
                String body;
                try {
                    int code = connection.getResponseCode();
                    InputStream stream = code >= 200 && code < 300
                            ? connection.getInputStream()
                            : connection.getErrorStream();
                    body = readAll(stream);
                    if (code < 200 || code >= 300) {
                        throw new IOException("Hugging Face returned HTTP " + code + ": " + body);
                    }
                } finally {
                    connection.disconnect();
                }
                JSONObject model = new JSONObject(body);
                JSONArray siblings = model.optJSONArray("siblings");
                JSONArray files = new JSONArray();
                if (siblings != null) {
                    for (int i = 0; i < siblings.length(); i++) {
                        JSONObject sibling = siblings.optJSONObject(i);
                        if (sibling == null) continue;
                        String name = sibling.optString("rfilename");
                        String kind = fileKind(name);
                        if ("other".equals(kind)) continue;
                        JSONObject file = new JSONObject();
                        file.put("name", name);
                        file.put("kind", kind);
                        long size = sibling.optLong("size", 0);
                        if (size <= 0) {
                            JSONObject lfs = sibling.optJSONObject("lfs");
                            if (lfs != null) size = lfs.optLong("size", 0);
                        }
                        file.put("size", size);
                        files.put(file);
                    }
                }
                JSONObject response = new JSONObject();
                response.put("ok", true);
                response.put("repo", repo);
                response.put("files", files);
                postJs("window.sllmHfFiles(" + response + ")");
            } catch (Exception error) {
                postJs("window.sllmHfFiles({ok:false,repo:" + JSONObject.quote(repo)
                        + ",error:" + JSONObject.quote(String.valueOf(error.getMessage())) + "})");
            }
        }, "sllm-hf-files");
        hfThread.start();
    }

    private void startHfDownload(String requestJson) {
        final JSONObject request;
        try {
            request = new JSONObject(requestJson);
        } catch (Exception error) {
            postJs("window.sllmHfDone({ok:false,error:" + JSONObject.quote("bad request") + "})");
            return;
        }
        cancelHf();
        hfCancel = false;
        hfThread = new Thread(() -> {
            FileOutputStream fileOut = null;
            HttpURLConnection connection = null;
            try {
                String repo = request.optString("repo", "");
                String file = request.optString("file", "");
                if (repo.isEmpty() || file.isEmpty()) {
                    throw new IOException("repo or file is empty");
                }
                String safeRepo = repo.replace("/", "_").replaceAll("[^A-Za-z0-9._-]", "-");
                String safeFile = file.replace("/", "_").replaceAll("[^A-Za-z0-9._-]", "-");
                File dir = new File(modelsDir(), "hf/" + safeRepo);
                if (!dir.exists() && !dir.mkdirs()) {
                    throw new IOException("cannot create " + dir);
                }
                File part = new File(dir, safeFile + ".part");
                File target = new File(dir, safeFile);
                long offset = part.isFile() ? part.length() : 0;

                String url = "https://huggingface.co/" + repo + "/resolve/main/" + file;
                connection = hfOpen(url, "application/octet-stream", true, offset);
                hfConnection = connection;
                int code = connection.getResponseCode();
                if (code == 416 && part.isFile()) {
                    if (!part.renameTo(target)) {
                        throw new IOException("cannot finalize " + target);
                    }
                    postJs("window.sllmHfDone({ok:true,repo:" + JSONObject.quote(repo)
                            + ",file:" + JSONObject.quote(file)
                            + ",path:" + JSONObject.quote(target.getAbsolutePath()) + "})");
                    refreshModels();
                    return;
                }
                if (code != 200 && code != 206) {
                    throw new IOException("Hugging Face returned HTTP " + code);
                }
                if (code == 200) {
                    offset = 0;
                    if (part.exists() && !part.delete()) {
                        throw new IOException("cannot restart " + part);
                    }
                }
                long length = connection.getContentLengthLong();
                long total = length > 0 ? offset + length : 0;
                StatFs stat = new StatFs(dir.getAbsolutePath());
                if (total > 0 && total > stat.getAvailableBytes() - 128L * 1024 * 1024) {
                    throw new IOException("not enough free space for " + file);
                }

                fileOut = new FileOutputStream(part, offset > 0);
                InputStream input = new BufferedInputStream(connection.getInputStream());
                byte[] buffer = new byte[64 * 1024];
                long received = offset;
                long lastPost = 0;
                int read;
                while ((read = input.read(buffer)) >= 0) {
                    if (hfCancel) {
                        throw new IOException("canceled");
                    }
                    fileOut.write(buffer, 0, read);
                    received += read;
                    long now = System.currentTimeMillis();
                    if (now - lastPost > 400) {
                        lastPost = now;
                        JSONObject progress = new JSONObject();
                        progress.put("repo", repo);
                        progress.put("file", file);
                        progress.put("received", received);
                        progress.put("total", total);
                        progress.put("state", "downloading");
                        postJs("window.sllmHfProgress(" + progress + ")");
                    }
                }
                fileOut.flush();
                fileOut.close();
                fileOut = null;
                if (!part.renameTo(target)) {
                    throw new IOException("cannot finalize " + target);
                }
                JSONObject done = new JSONObject();
                done.put("ok", true);
                done.put("repo", repo);
                done.put("file", file);
                done.put("path", target.getAbsolutePath());
                done.put("bytes", received);
                postJs("window.sllmHfDone(" + done + ")");
                refreshModels();
            } catch (Exception error) {
                boolean canceled = hfCancel;
                postJs("window.sllmHfDone({ok:false,canceled:" + canceled
                        + ",error:" + JSONObject.quote(String.valueOf(error.getMessage())) + "})");
            } finally {
                hfConnection = null;
                if (fileOut != null) {
                    try { fileOut.close(); } catch (Exception ignored) {}
                }
                if (connection != null) connection.disconnect();
            }
        }, "sllm-hf-download");
        hfThread.start();
    }

    private int availMb() {
        ActivityManager am = (ActivityManager) getSystemService(Context.ACTIVITY_SERVICE);
        ActivityManager.MemoryInfo info = new ActivityManager.MemoryInfo();
        am.getMemoryInfo(info);
        return (int) (info.availMem / (1024L * 1024L));
    }

    private void refreshModels() {
        new Thread(() -> {
            String json = nativeModels(modelsDir(), availMb());
            postJs("(function(){try{window.sllmModels&&window.sllmModels("
                    + json + ");return 'ok';}catch(e){return 'err:'+e.message;}})()");
        }, "sllm-models").start();
    }

    private void cancelHf() {
        hfCancel = true;
        HttpURLConnection connection = hfConnection;
        if (connection != null) {
            connection.disconnect();
        }
    }

    public final class Bridge {
        @JavascriptInterface
        public String engine() {
            return nativeEngine();
        }

        /** Start a generation. The worker drains `poll()` for events. */
        @JavascriptInterface
        public String start(String requestJson) {
            return nativeStart(requestJson, modelsDir());
        }

        @JavascriptInterface
        public String poll() {
            return nativePoll();
        }

        /** Preload a model; progress is reported as `model` events via poll(). */
        @JavascriptInterface
        public String prepare(String modelId, String modelPath) {
            return nativePrepare(modelId, modelPath, modelsDir());
        }

        @JavascriptInterface
        public void hfSearch(String requestJson) {
            startHfSearch(requestJson);
        }

        @JavascriptInterface
        public void hfFiles(String repo) {
            startHfFiles(repo);
        }

        @JavascriptInterface
        public void hfDownload(String requestJson) {
            startHfDownload(requestJson);
        }

        @JavascriptInterface
        public void hfCancel() {
            cancelHf();
        }

        @JavascriptInterface
        public String hfToken() {
            return MainActivity.this.hfToken();
        }

        @JavascriptInterface
        public void setHfToken(String token) {
            getSharedPreferences("sllm", MODE_PRIVATE)
                    .edit()
                    .putString("hf.token", token == null ? "" : token.trim())
                    .apply();
        }

        @JavascriptInterface
        public void cancel() {
            nativeCancel();
        }

        /** New chat / abort: cancel the current generation and drop its queue. */
        @JavascriptInterface
        public void reset() {
            nativeCancel();
        }

        @JavascriptInterface
        public String stats() {
            return nativeStats();
        }

        @JavascriptInterface
        public void unload() {
            nativeUnload();
        }

        @JavascriptInterface
        public String theme() {
            return resolveTheme(readThemePref());
        }

        @JavascriptInterface
        public String themePref() {
            return readThemePref();
        }

        @JavascriptInterface
        public void setTheme(String pref) {
            switch (pref) {
                case "system":
                case "light":
                case "dark":
                case "night":
                case "amoled":
                    break;
                default:
                    return;
            }
            getSharedPreferences("sllm", MODE_PRIVATE)
                    .edit()
                    .putString("theme", pref)
                    .apply();
            runOnUiThread(() -> applyWindowTheme(pref));
        }

        @JavascriptInterface
        public String version() {
            return nativeVersion();
        }

        @JavascriptInterface
        public void loadModels() {
            refreshModels();
        }

        @JavascriptInterface
        public String models() {
            File base = getExternalFilesDir(null);
            if (base == null) {
                base = getFilesDir();
            }
            File dir = new File(base, "models");
            ActivityManager am = (ActivityManager) getSystemService(Context.ACTIVITY_SERVICE);
            ActivityManager.MemoryInfo info = new ActivityManager.MemoryInfo();
            am.getMemoryInfo(info);
            int availMb = (int) (info.availMem / (1024L * 1024L));
            return nativeModels(dir.getAbsolutePath(), availMb);
        }

        /** llama.cpp version string read from the linked library. */
        @JavascriptInterface
        public String llamaVersion() {
            return nativeLlamaVersion();
        }

        /**
         * Debug-only: run the real llama.cpp smoke test. Returns immediately;
         * the result is posted to window.sllmSmoke and logged under tag SLLM.
         */
        @JavascriptInterface
        public void runLlamaSmoke(String modelPath) {
            startLlamaSmoke(modelPath);
        }
    }
}
