import { render } from "preact";
import { App } from "./app";
import "@fontsource/inter/latin-400.css";
import "@fontsource/inter/latin-500.css";
import "@fontsource/inter/latin-600.css";
import "@fontsource/inter/cyrillic-400.css";
import "@fontsource/inter/cyrillic-500.css";
import "@fontsource/inter/cyrillic-600.css";
import "@fontsource/newsreader/latin-400.css";
import "@fontsource/newsreader/latin-500.css";
import "@fontsource/ibm-plex-mono/latin-500.css";
import "./styles.css";

render(<App />, document.getElementById("app")!);

// The shell is interactive after this frame. Keep the splash short: it exists
// to cover WebView cold start, not to delay the app.
requestAnimationFrame(() => {
  const splash = document.getElementById("lumen-splash");
  if (!splash) return;
  splash.classList.add("is-hidden");
  window.setTimeout(() => splash.remove(), 380);
});
