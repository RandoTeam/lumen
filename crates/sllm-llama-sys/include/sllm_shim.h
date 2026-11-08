#ifndef SLLM_SHIM_H
#define SLLM_SHIM_H

/*
 * C ABI for the SnapdragonLLM shim around the pinned llama.cpp fork.
 *
 * Every function catches C++ exceptions. Integer-returning functions return
 * SLLM_SHIM_OK (0) on success and one of the negative codes below on failure.
 * Functions that mirror a llama.cpp call keep its return convention (for
 * example llm_tokenize returns the required token count as a negative value
 * when the buffer is too small).
 */

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

#define SLLM_SHIM_OK 0
#define SLLM_SHIM_ERR_ARG -1
#define SLLM_SHIM_ERR_OOM -2
#define SLLM_SHIM_ERR_EXCEPTION -3
#define SLLM_SHIM_ERR_STATE -4

typedef struct sllm_model sllm_model;
typedef struct sllm_context sllm_context;
typedef struct sllm_sampler sllm_sampler;

typedef struct sllm_model_options {
    int32_t n_gpu_layers;
    int32_t load_mode;        /* enum llama_load_mode: -1 auto, 0 none, 1 mmap, 2 mlock, 3 mmap+mlock, 4 direct_io */
} sllm_model_options;

typedef struct sllm_context_options {
    uint32_t n_ctx;
    uint32_t n_batch;
    uint32_t n_ubatch;
    int32_t n_threads;
    int32_t n_threads_batch;
    int32_t type_k;           /* enum ggml_type */
    int32_t type_v;           /* enum ggml_type */
    bool offload_kqv;
    int32_t flash_attn_type;  /* enum llama_flash_attn_type */
} sllm_context_options;

/* Version of the linked llama.cpp build (static const string). */
const char * sllm_version(void);

/* Last error message for the calling thread, or an empty string. */
const char * sllm_last_error(void);

/* Optional one-time backend initialisation. Safe to call more than once. */
void sllm_backend_init(void);
void sllm_backend_free(void);

int32_t sllm_model_default_options(sllm_model_options * out);
int32_t sllm_context_default_options(sllm_context_options * out);

int32_t sllm_model_load(const char * path, const sllm_model_options * options, sllm_model ** out);
void sllm_model_free(sllm_model * model);

/* Returns the number of bytes written, excluding the terminator, or a
 * negative required size, or a negative SLLM_SHIM_ERR_* code. */
int32_t sllm_model_desc(const sllm_model * model, char * buf, int32_t buf_len);

/* Chat template source stored in the model, or NULL when the file has none.
 * The pointer is owned by the model and valid until it is freed. */
const char * sllm_model_chat_template(const sllm_model * model);

int32_t sllm_model_n_ctx_train(const sllm_model * model);

/* Apply a chat template. `tmpl` may be NULL for the built-in chatml template.
 * roles/contents hold `n_msg` UTF-8 strings. Returns the required byte count
 * (excluding the terminator) or a negative SLLM_SHIM_ERR_* code. When `buf` is
 * large enough the formatted prompt is written and NUL terminated. */
int32_t sllm_chat_apply_template(const char * tmpl,
                                 const char * const * roles,
                                 const char * const * contents,
                                 int32_t n_msg, bool add_ass,
                                 char * buf, int32_t buf_len);

int32_t sllm_context_new(sllm_model * model, const sllm_context_options * options, sllm_context ** out);
void sllm_context_free(sllm_context * ctx);

/* Load a rank-r LoRA adapter and attach it to the context at `scale`.
 * Adapters stay active until the context is freed. */
int32_t sllm_context_apply_lora(sllm_context * ctx, const char * path, float scale);

/* Drop KV cache and recurrent state for all sequences, keeping the context
 * allocated. Call before each independent generation. */
void sllm_context_clear_memory(sllm_context * ctx);

uint32_t sllm_context_n_ctx(const sllm_context * ctx);
int32_t sllm_context_n_vocab(const sllm_context * ctx);

/* Mirrors llama_tokenize: token count, or negative required count. */
int32_t sllm_tokenize(sllm_context * ctx, const char * text, int32_t text_len,
                      int32_t * tokens, int32_t n_tokens_max,
                      bool add_special, bool parse_special);

/* Mirrors llama_token_to_piece: byte count, or negative required count. */
int32_t sllm_token_to_piece(sllm_context * ctx, int32_t token, char * buf, int32_t buf_len,
                            int32_t lstrip, bool special);

/* Mirrors llama_detokenize: byte count, or negative required count. */
int32_t sllm_detokenize(sllm_context * ctx, const int32_t * tokens, int32_t n_tokens,
                        char * buf, int32_t buf_len, bool remove_special, bool unparse_special);

/* One-sequence decode. start_pos < 0 lets llama.cpp track positions.
 * Returns the llama_decode result (0 = ok, > 0 warning, < 0 error). */
int32_t sllm_decode(sllm_context * ctx, const int32_t * tokens, int32_t n_tokens,
                    int32_t start_pos, int32_t seq_id, bool logits_all);

/* Logits for the last output (idx -1). Valid until the next decode. */
const float * sllm_get_logits(sllm_context * ctx);

/* Vocabulary helpers. */
bool sllm_vocab_is_eog(const sllm_context * ctx, int32_t token);
int32_t sllm_vocab_eos(const sllm_context * ctx);
int32_t sllm_vocab_bos(const sllm_context * ctx);
bool sllm_vocab_get_add_bos(const sllm_context * ctx);

/* Greedy when temp <= 0, otherwise penalties -> top_k -> top_p -> min_p ->
 * temp -> dist. */
sllm_sampler * sllm_sampler_new(int32_t top_k, float top_p, float min_p, float temp, uint32_t seed);

/* Same chain with a repetition-penalty stage. `penalty_last_n == 0` or
 * `n_vocab <= 0` disables penalties. */
sllm_sampler * sllm_sampler_new_ex(int32_t top_k, float top_p, float min_p, float temp,
                                   int32_t n_vocab, float repeat_penalty,
                                   int32_t penalty_last_n, uint32_t seed);
void sllm_sampler_free(sllm_sampler * smpl);
int32_t sllm_sampler_sample(sllm_sampler * smpl, sllm_context * ctx);
void sllm_sampler_accept(sllm_sampler * smpl, int32_t token);
void sllm_sampler_reset(sllm_sampler * smpl);

#ifdef __cplusplus
} /* extern "C" */
#endif

#endif /* SLLM_SHIM_H */
