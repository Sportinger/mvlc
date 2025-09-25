#pragma once

#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

struct pl_context;
struct pl_renderer;

struct pl_context *pl_context_create(void);
void pl_context_destroy(struct pl_context *ctx);

struct pl_renderer *pl_renderer_create(struct pl_context *ctx);
void pl_renderer_destroy(struct pl_renderer *renderer);

int pl_renderer_test_quad(struct pl_renderer *renderer);
int pl_renderer_test_quad_rgba(struct pl_renderer *renderer,
                               uint32_t width,
                               uint32_t height,
                               uint8_t *dst,
                               uint32_t stride);

#ifdef __cplusplus
}
#endif
