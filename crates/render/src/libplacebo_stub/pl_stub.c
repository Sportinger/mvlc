#include "pl_stub.h"

#include <stdlib.h>
#include <string.h>

struct pl_context {
    int placeholder;
};

struct pl_renderer {
    struct pl_context *ctx;
};

struct pl_context *pl_context_create(void) {
    struct pl_context *ctx = (struct pl_context *)malloc(sizeof(struct pl_context));
    if (ctx) {
        ctx->placeholder = 0;
    }
    return ctx;
}

void pl_context_destroy(struct pl_context *ctx) {
    free(ctx);
}

struct pl_renderer *pl_renderer_create(struct pl_context *ctx) {
    if (!ctx) {
        return NULL;
    }

    struct pl_renderer *renderer = (struct pl_renderer *)malloc(sizeof(struct pl_renderer));
    if (renderer) {
        renderer->ctx = ctx;
    }
    return renderer;
}

void pl_renderer_destroy(struct pl_renderer *renderer) {
    free(renderer);
}

int pl_renderer_test_quad(struct pl_renderer *renderer) {
    (void)renderer;
    return 1;
}

int pl_renderer_test_quad_rgba(struct pl_renderer *renderer,
                               uint32_t width,
                               uint32_t height,
                               uint8_t *dst,
                               uint32_t stride) {
    if (!renderer || !dst || width == 0 || height == 0) {
        return 0;
    }

    for (uint32_t y = 0; y < height; ++y) {
        uint8_t *row = dst + (size_t)y * stride;
        for (uint32_t x = 0; x < width; ++x) {
            uint8_t *px = row + (size_t)x * 4;
            int top = (int)(y < height / 2);
            int left = (int)(x < width / 2);

            if (top && left) {
                // top-left red
                px[0] = 255;
                px[1] = 0;
                px[2] = 0;
            } else if (top && !left) {
                // top-right green
                px[0] = 0;
                px[1] = 255;
                px[2] = 0;
            } else if (!top && left) {
                // bottom-left blue
                px[0] = 0;
                px[1] = 0;
                px[2] = 255;
            } else {
                // bottom-right white
                px[0] = 255;
                px[1] = 255;
                px[2] = 255;
            }
            px[3] = 255;
        }
    }

    return 1;
}
