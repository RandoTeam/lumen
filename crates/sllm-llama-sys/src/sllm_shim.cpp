// C ABI shim around the pinned llama.cpp fork.
//
// The shim exists for two reasons:
//  - keep C++ exceptions inside this translation unit,
//  - expose a small, stable surface instead of the full C API to Rust.
//
// Every entry point catches exceptions and reports an error message that Rust
// can read through sllm_last_error().

#include "sllm_shim.h"

#include "llama.h"

#include <cstdio>
#include <cstring>
#include <exception>
#include <new>
#include <string>
#include <utility>
#include <vector>

struct sllm_model {
    llama_model * model = nullptr;
};

struct sllm_context {
    llama_context * ctx = nullptr;
    const llama_vocab * vocab = nullptr;
    llama_model * model = nullptr;
    std::vector<llama_adapter_lora *> adapters;
    std::vector<float> adapter_scales;
};

struct sllm_sampler {
    llama_sampler * smpl = nullptr;
};

namespace {

// Fixed buffer: error reporting must not allocate, otherwise a bad_alloc while
// reporting a failure could escape the extern "C" boundary.
thread_local char g_last_error[1024] = {};

void clear_error() {
    g_last_error[0] = '\0';
}

void set_error(const char * message) {
    std::snprintf(g_last_error, sizeof(g_last_error), "%s", message ? message : "");
}

void set_error(const std::string & message) {
    set_error(message.c_str());
}

template <typename F>
int32_t guard_int(F && fn) noexcept {
    try {
        clear_error();
        return fn();
    } catch (const std::bad_alloc &) {
        set_error("out of memory");
        return SLLM_SHIM_ERR_OOM;
    } catch (const std::exception & e) {
        set_error(e.what());
        return SLLM_SHIM_ERR_EXCEPTION;
    } catch (...) {
        set_error("unknown C++ exception");
        return SLLM_SHIM_ERR_EXCEPTION;
    }
}

template <typename F>
void guard_void(F && fn) noexcept {
    try {
        clear_error();
        fn();
    } catch (const std::bad_alloc &) {
        set_error("out of memory");
    } catch (const std::exception & e) {
        set_error(e.what());
    } catch (...) {
        set_error("unknown C++ exception");
    }
}

template <typename T, typename F>
T * guard_ptr(F && fn, const char * what) noexcept {
    try {
        clear_error();
        T * value = fn();
        if (!value) {
            set_error(what);
        }
        return value;
    } catch (const std::bad_alloc &) {
        set_error("out of memory");
        return nullptr;
    } catch (const std::exception & e) {
        set_error(e.what());
        return nullptr;
    } catch (...) {
        set_error("unknown C++ exception");
        return nullptr;
    }
}

bool valid_model(const sllm_model * model) {
    return model && model->model;
}

bool valid_context(const sllm_context * ctx) {
    return ctx && ctx->ctx && ctx->vocab;
}

bool valid_sampler(const sllm_sampler * smpl) {
    return smpl && smpl->smpl;
}

} // namespace

extern "C" {

const char * sllm_version(void) {
    return llama_version();
}

const char * sllm_last_error(void) {
    return g_last_error;
}

void sllm_backend_init(void) {
    guard_void([] { llama_backend_init(); });
}

void sllm_backend_free(void) {
    guard_void([] { llama_backend_free(); });
}

int32_t sllm_model_default_options(sllm_model_options * out) {
    if (!out) {
        set_error("sllm_model_default_options: out is NULL");
        return SLLM_SHIM_ERR_ARG;
    }
    return guard_int([&]() -> int32_t {
        const llama_model_params params = llama_model_default_params();
        out->n_gpu_layers = params.n_gpu_layers;
        out->load_mode    = static_cast<int32_t>(params.load_mode);
        return SLLM_SHIM_OK;
    });
}

int32_t sllm_context_default_options(sllm_context_options * out) {
    if (!out) {
        set_error("sllm_context_default_options: out is NULL");
        return SLLM_SHIM_ERR_ARG;
    }
    return guard_int([&]() -> int32_t {
        const llama_context_params params = llama_context_default_params();
        out->n_ctx            = params.n_ctx;
        out->n_batch          = params.n_batch;
        out->n_ubatch         = params.n_ubatch;
        out->n_threads        = params.n_threads;
        out->n_threads_batch  = params.n_threads_batch;
        out->type_k           = static_cast<int32_t>(params.type_k);
        out->type_v           = static_cast<int32_t>(params.type_v);
        out->offload_kqv      = params.offload_kqv;
        out->flash_attn_type  = static_cast<int32_t>(params.flash_attn_type);
        return SLLM_SHIM_OK;
    });
}

int32_t sllm_model_load(const char * path, const sllm_model_options * options, sllm_model ** out) {
    if (!out) {
        set_error("sllm_model_load: out is NULL");
        return SLLM_SHIM_ERR_ARG;
    }
    *out = nullptr;
    if (!path || path[0] == '\0') {
        set_error("sllm_model_load: path is empty");
        return SLLM_SHIM_ERR_ARG;
    }

    return guard_int([&]() -> int32_t {
        llama_model_params params = llama_model_default_params();
        if (options) {
            params.n_gpu_layers = options->n_gpu_layers;
            params.load_mode    = static_cast<llama_load_mode>(options->load_mode);
        }

        llama_model * model = llama_model_load_from_file(path, params);
        if (!model) {
            set_error("llama_model_load_from_file failed; check llama.cpp log output");
            return SLLM_SHIM_ERR_STATE;
        }

        sllm_model * wrapper = nullptr;
        try {
            wrapper = new sllm_model{};
        } catch (...) {
            llama_model_free(model);
            throw;
        }
        wrapper->model = model;
        *out = wrapper;
        return SLLM_SHIM_OK;
    });
}

void sllm_model_free(sllm_model * model) {
    if (!model) {
        return;
    }
    guard_void([&] {
        if (model->model) {
            llama_model_free(model->model);
            model->model = nullptr;
        }
    });
    delete model;
}

int32_t sllm_model_desc(const sllm_model * model, char * buf, int32_t buf_len) {
    if (!valid_model(model)) {
        set_error("sllm_model_desc: invalid model");
        return SLLM_SHIM_ERR_ARG;
    }
    if (!buf || buf_len <= 0) {
        set_error("sllm_model_desc: invalid buffer");
        return SLLM_SHIM_ERR_ARG;
    }
    return guard_int([&]() -> int32_t {
        return llama_model_desc(model->model, buf, static_cast<size_t>(buf_len));
    });
}

const char * sllm_model_chat_template(const sllm_model * model) {
    if (!valid_model(model)) {
        set_error("sllm_model_chat_template: invalid model");
        return nullptr;
    }
    clear_error();
    return llama_model_chat_template(model->model, nullptr);
}

int32_t sllm_model_n_ctx_train(const sllm_model * model) {
    if (!valid_model(model)) {
        set_error("sllm_model_n_ctx_train: invalid model");
        return SLLM_SHIM_ERR_ARG;
    }
    return guard_int([&]() -> int32_t { return llama_model_n_ctx_train(model->model); });
}

int32_t sllm_chat_apply_template(const char * tmpl,
                                 const char * const * roles,
                                 const char * const * contents,
                                 int32_t n_msg, bool add_ass,
                                 char * buf, int32_t buf_len) {
    if (n_msg < 0 || (n_msg > 0 && (!roles || !contents))) {
        set_error("sllm_chat_apply_template: invalid message arrays");
        return SLLM_SHIM_ERR_ARG;
    }
    if (buf_len < 0 || (buf_len > 0 && !buf)) {
        set_error("sllm_chat_apply_template: invalid buffer");
        return SLLM_SHIM_ERR_ARG;
    }

    return guard_int([&]() -> int32_t {
        std::vector<llama_chat_message> chat;
        chat.reserve(static_cast<size_t>(n_msg));
        for (int32_t i = 0; i < n_msg; ++i) {
            if (!roles[i] || !contents[i]) {
                set_error("sllm_chat_apply_template: NULL role or content");
                return SLLM_SHIM_ERR_ARG;
            }
            chat.push_back({ roles[i], contents[i] });
        }

        const int32_t need = llama_chat_apply_template(tmpl, chat.data(), chat.size(),
                                                       add_ass, nullptr, 0);
        if (need < 0) {
            set_error("llama_chat_apply_template does not recognise this template");
            return SLLM_SHIM_ERR_STATE;
        }
        if (buf && buf_len > need) {
            const int32_t written = llama_chat_apply_template(tmpl, chat.data(), chat.size(),
                                                              add_ass, buf, buf_len);
            if (written < 0) {
                set_error("llama_chat_apply_template failed while writing");
                return SLLM_SHIM_ERR_STATE;
            }
            buf[need] = '\0';
        }
        return need;
    });
}

int32_t sllm_context_new(sllm_model * model, const sllm_context_options * options, sllm_context ** out) {
    if (!out) {
        set_error("sllm_context_new: out is NULL");
        return SLLM_SHIM_ERR_ARG;
    }
    *out = nullptr;
    if (!valid_model(model)) {
        set_error("sllm_context_new: invalid model");
        return SLLM_SHIM_ERR_ARG;
    }

    return guard_int([&]() -> int32_t {
        llama_context_params params = llama_context_default_params();
        if (options) {
            params.n_ctx           = options->n_ctx;
            params.n_batch         = options->n_batch;
            params.n_ubatch        = options->n_ubatch;
            params.n_threads       = options->n_threads;
            params.n_threads_batch = options->n_threads_batch;
            params.type_k          = static_cast<ggml_type>(options->type_k);
            params.type_v          = static_cast<ggml_type>(options->type_v);
            params.offload_kqv     = options->offload_kqv;
            params.flash_attn_type = static_cast<llama_flash_attn_type>(options->flash_attn_type);
        }

        llama_context * ctx = llama_init_from_model(model->model, params);
        if (!ctx) {
            set_error("llama_init_from_model failed; check llama.cpp log output");
            return SLLM_SHIM_ERR_STATE;
        }

        sllm_context * wrapper = nullptr;
        try {
            wrapper = new sllm_context{};
        } catch (...) {
            llama_free(ctx);
            throw;
        }
        wrapper->ctx   = ctx;
        wrapper->vocab = llama_model_get_vocab(model->model);
        wrapper->model = model->model;
        *out = wrapper;
        return SLLM_SHIM_OK;
    });
}

int32_t sllm_context_apply_lora(sllm_context * ctx, const char * path, float scale) {
    if (!valid_context(ctx)) {
        set_error("sllm_context_apply_lora: invalid context");
        return SLLM_SHIM_ERR_ARG;
    }
    if (!ctx->model) {
        set_error("sllm_context_apply_lora: context has no model");
        return SLLM_SHIM_ERR_ARG;
    }
    if (!path || path[0] == '\0') {
        set_error("sllm_context_apply_lora: path is empty");
        return SLLM_SHIM_ERR_ARG;
    }

    return guard_int([&]() -> int32_t {
        llama_adapter_lora * adapter = llama_adapter_lora_init(ctx->model, path);
        if (!adapter) {
            set_error("llama_adapter_lora_init failed");
            return SLLM_SHIM_ERR_STATE;
        }

        ctx->adapters.push_back(adapter);
        ctx->adapter_scales.push_back(scale);

        const int32_t rc = llama_set_adapters_lora(
            ctx->ctx,
            ctx->adapters.data(),
            ctx->adapters.size(),
            ctx->adapter_scales.data());
        if (rc != 0) {
            ctx->adapters.pop_back();
            ctx->adapter_scales.pop_back();
            llama_adapter_lora_free(adapter);
            set_error("llama_set_adapters_lora returned " + std::to_string(rc));
            return SLLM_SHIM_ERR_STATE;
        }
        return SLLM_SHIM_OK;
    });
}

void sllm_context_free(sllm_context * ctx) {
    if (!ctx) {
        return;
    }
    guard_void([&] {
        if (ctx->ctx) {
            llama_free(ctx->ctx);
            ctx->ctx = nullptr;
        }
        for (auto * adapter : ctx->adapters) {
            llama_adapter_lora_free(adapter);
        }
        ctx->adapters.clear();
        ctx->adapter_scales.clear();
        ctx->vocab = nullptr;
        ctx->model = nullptr;
    });
    delete ctx;
}

void sllm_context_clear_memory(sllm_context * ctx) {
    if (!valid_context(ctx)) {
        set_error("sllm_context_clear_memory: invalid context");
        return;
    }
    guard_void([&] { llama_memory_clear(llama_get_memory(ctx->ctx), true); });
}

uint32_t sllm_context_n_ctx(const sllm_context * ctx) {
    if (!valid_context(ctx)) {
        set_error("sllm_context_n_ctx: invalid context");
        return 0;
    }
    return llama_n_ctx(ctx->ctx);
}

int32_t sllm_context_n_vocab(const sllm_context * ctx) {
    if (!valid_context(ctx)) {
        set_error("sllm_context_n_vocab: invalid context");
        return SLLM_SHIM_ERR_ARG;
    }
    return llama_vocab_n_tokens(ctx->vocab);
}

int32_t sllm_tokenize(sllm_context * ctx, const char * text, int32_t text_len,
                      int32_t * tokens, int32_t n_tokens_max,
                      bool add_special, bool parse_special) {
    if (!valid_context(ctx)) {
        set_error("sllm_tokenize: invalid context");
        return SLLM_SHIM_ERR_ARG;
    }
    if (!text && text_len > 0) {
        set_error("sllm_tokenize: text is NULL");
        return SLLM_SHIM_ERR_ARG;
    }
    if (text_len < 0) {
        text_len = text ? static_cast<int32_t>(std::strlen(text)) : 0;
    }
    if (n_tokens_max < 0 || (n_tokens_max > 0 && !tokens)) {
        set_error("sllm_tokenize: invalid token buffer");
        return SLLM_SHIM_ERR_ARG;
    }

    return guard_int([&]() -> int32_t {
        return llama_tokenize(ctx->vocab, text, text_len, tokens, n_tokens_max, add_special, parse_special);
    });
}

int32_t sllm_token_to_piece(sllm_context * ctx, int32_t token, char * buf, int32_t buf_len,
                            int32_t lstrip, bool special) {
    if (!valid_context(ctx)) {
        set_error("sllm_token_to_piece: invalid context");
        return SLLM_SHIM_ERR_ARG;
    }
    if (buf_len < 0 || (buf_len > 0 && !buf)) {
        set_error("sllm_token_to_piece: invalid buffer");
        return SLLM_SHIM_ERR_ARG;
    }

    return guard_int([&]() -> int32_t {
        return llama_token_to_piece(ctx->vocab, token, buf, buf_len, lstrip, special);
    });
}

int32_t sllm_detokenize(sllm_context * ctx, const int32_t * tokens, int32_t n_tokens,
                        char * buf, int32_t buf_len, bool remove_special, bool unparse_special) {
    if (!valid_context(ctx)) {
        set_error("sllm_detokenize: invalid context");
        return SLLM_SHIM_ERR_ARG;
    }
    if (n_tokens < 0 || (n_tokens > 0 && !tokens)) {
        set_error("sllm_detokenize: invalid tokens");
        return SLLM_SHIM_ERR_ARG;
    }
    if (buf_len < 0 || (buf_len > 0 && !buf)) {
        set_error("sllm_detokenize: invalid buffer");
        return SLLM_SHIM_ERR_ARG;
    }

    return guard_int([&]() -> int32_t {
        return llama_detokenize(ctx->vocab, tokens, n_tokens, buf, buf_len, remove_special, unparse_special);
    });
}

int32_t sllm_decode(sllm_context * ctx, const int32_t * tokens, int32_t n_tokens,
                    int32_t start_pos, int32_t seq_id, bool logits_all) {
    if (!valid_context(ctx)) {
        set_error("sllm_decode: invalid context");
        return SLLM_SHIM_ERR_ARG;
    }
    if (n_tokens <= 0 || !tokens) {
        set_error("sllm_decode: invalid tokens");
        return SLLM_SHIM_ERR_ARG;
    }
    if (seq_id < 0) {
        set_error("sllm_decode: invalid seq_id");
        return SLLM_SHIM_ERR_ARG;
    }

    return guard_int([&]() -> int32_t {
        llama_batch batch = llama_batch_init(n_tokens, 0, 1);
        batch.n_tokens = n_tokens;
        for (int32_t i = 0; i < n_tokens; ++i) {
            batch.token[i]      = static_cast<llama_token>(tokens[i]);
            batch.pos[i]        = start_pos < 0 ? i : start_pos + i;
            batch.n_seq_id[i]   = 1;
            batch.seq_id[i][0]  = static_cast<llama_seq_id>(seq_id);
            batch.logits[i]     = (logits_all || i == n_tokens - 1) ? 1 : 0;
        }

        const int32_t rc = llama_decode(ctx->ctx, batch);
        llama_batch_free(batch);

        if (rc != 0) {
            set_error("llama_decode returned " + std::to_string(rc));
        }
        return rc;
    });
}

const float * sllm_get_logits(sllm_context * ctx) {
    if (!valid_context(ctx)) {
        set_error("sllm_get_logits: invalid context");
        return nullptr;
    }
    return llama_get_logits_ith(ctx->ctx, -1);
}

bool sllm_vocab_is_eog(const sllm_context * ctx, int32_t token) {
    if (!valid_context(ctx)) {
        set_error("sllm_vocab_is_eog: invalid context");
        return false;
    }
    return llama_vocab_is_eog(ctx->vocab, static_cast<llama_token>(token));
}

int32_t sllm_vocab_eos(const sllm_context * ctx) {
    if (!valid_context(ctx)) {
        set_error("sllm_vocab_eos: invalid context");
        return SLLM_SHIM_ERR_ARG;
    }
    return static_cast<int32_t>(llama_vocab_eos(ctx->vocab));
}

int32_t sllm_vocab_bos(const sllm_context * ctx) {
    if (!valid_context(ctx)) {
        set_error("sllm_vocab_bos: invalid context");
        return SLLM_SHIM_ERR_ARG;
    }
    return static_cast<int32_t>(llama_vocab_bos(ctx->vocab));
}

bool sllm_vocab_get_add_bos(const sllm_context * ctx) {
    if (!valid_context(ctx)) {
        set_error("sllm_vocab_get_add_bos: invalid context");
        return false;
    }
    return llama_vocab_get_add_bos(ctx->vocab);
}

sllm_sampler * sllm_sampler_new(int32_t top_k, float top_p, float min_p, float temp, uint32_t seed) {
    return sllm_sampler_new_ex(top_k, top_p, min_p, temp, 0, 1.0f, 0, seed);
}

sllm_sampler * sllm_sampler_new_ex(int32_t top_k, float top_p, float min_p, float temp,
                                   int32_t n_vocab, float repeat_penalty,
                                   int32_t penalty_last_n, uint32_t seed) {
    return guard_ptr<sllm_sampler>([&]() -> sllm_sampler * {
        llama_sampler * chain = llama_sampler_chain_init(llama_sampler_chain_default_params());
        if (!chain) {
            return nullptr;
        }

        auto add = [&](llama_sampler * smpl) {
            if (!smpl) {
                return false;
            }
            llama_sampler_chain_add(chain, smpl);
            return true;
        };

        bool ok = true;
        if (n_vocab > 0 && penalty_last_n > 0 && repeat_penalty > 0.0f && repeat_penalty != 1.0f) {
            ok = add(llama_sampler_init_penalties(n_vocab, penalty_last_n, repeat_penalty, 0.0f, 0.0f));
        }
        if (ok && top_k > 0) {
            ok = add(llama_sampler_init_top_k(top_k));
        }
        if (ok && top_p > 0.0f && top_p < 1.0f) {
            ok = add(llama_sampler_init_top_p(top_p, 1));
        }
        if (ok && min_p > 0.0f) {
            ok = add(llama_sampler_init_min_p(min_p, 1));
        }
        if (ok && temp >= 0.0f) {
            ok = add(llama_sampler_init_temp(temp));
        }
        if (ok) {
            ok = add(temp <= 0.0f ? llama_sampler_init_greedy() : llama_sampler_init_dist(seed));
        }

        if (!ok) {
            llama_sampler_free(chain);
            return nullptr;
        }

        auto * wrapper = new sllm_sampler{};
        wrapper->smpl = chain;
        return wrapper;
    }, "sllm_sampler_new: failed to create sampler");
}

void sllm_sampler_free(sllm_sampler * smpl) {
    if (!smpl) {
        return;
    }
    guard_void([&] {
        if (smpl->smpl) {
            llama_sampler_free(smpl->smpl);
            smpl->smpl = nullptr;
        }
    });
    delete smpl;
}

int32_t sllm_sampler_sample(sllm_sampler * smpl, sllm_context * ctx) {
    if (!valid_sampler(smpl)) {
        set_error("sllm_sampler_sample: invalid sampler");
        return SLLM_SHIM_ERR_ARG;
    }
    if (!valid_context(ctx)) {
        set_error("sllm_sampler_sample: invalid context");
        return SLLM_SHIM_ERR_ARG;
    }

    return guard_int([&]() -> int32_t {
        const llama_token token = llama_sampler_sample(smpl->smpl, ctx->ctx, -1);
        if (token == LLAMA_TOKEN_NULL) {
            set_error("llama_sampler_sample returned LLAMA_TOKEN_NULL");
            return SLLM_SHIM_ERR_STATE;
        }
        return static_cast<int32_t>(token);
    });
}

void sllm_sampler_accept(sllm_sampler * smpl, int32_t token) {
    if (!valid_sampler(smpl)) {
        set_error("sllm_sampler_accept: invalid sampler");
        return;
    }
    guard_void([&] { llama_sampler_accept(smpl->smpl, static_cast<llama_token>(token)); });
}

void sllm_sampler_reset(sllm_sampler * smpl) {
    if (!valid_sampler(smpl)) {
        set_error("sllm_sampler_reset: invalid sampler");
        return;
    }
    guard_void([&] { llama_sampler_reset(smpl->smpl); });
}

} // extern "C"
