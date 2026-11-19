#ifndef RMPD_DSP_PLUGIN_ABI_H
#define RMPD_DSP_PLUGIN_ABI_H

#include <stdint.h>
#include <stddef.h>

#ifdef __cplusplus
extern "C" {
#endif

#define RMPD_DSP_ABI_VERSION 1

typedef struct rmpd_dsp_plugin_t {
    uint32_t abi_version;
    const char *id;
    const char *name;
    void *(*create)(uint32_t sample_rate, uint16_t channels);
    void (*destroy)(void *instance);
    /**
     * Real-time audio process callback:
     * MUST NOT allocate heap memory, lock mutexes, or perform blocking I/O.
     */
    void (*process_f32)(void *instance, const float *in_interleaved, float *out_interleaved, size_t frames);
    void (*reset)(void *instance);
} rmpd_dsp_plugin_t;

typedef const rmpd_dsp_plugin_t *(*rmpd_dsp_entry_fn)(void);

#ifdef __cplusplus
}
#endif

#endif /* RMPD_DSP_PLUGIN_ABI_H */
