#include "qr-image.h"

#include <cairo.h>
#include <stdint.h>

#include "qrcodegen.h"

bool signal_native_qr_write_png(const char *text, const char *path) {
    uint8_t temp[qrcodegen_BUFFER_LEN_MAX];
    uint8_t code[qrcodegen_BUFFER_LEN_MAX];
    if (!qrcodegen_encodeText(text, temp, code, qrcodegen_Ecc_MEDIUM,
                              qrcodegen_VERSION_MIN, qrcodegen_VERSION_MAX,
                              qrcodegen_Mask_AUTO, true)) {
        return false;
    }

    const int size = qrcodegen_getSize(code);
    const int quiet = 4;
    const int scale = 8;
    const int pixels = (size + quiet * 2) * scale;
    cairo_surface_t *surface = cairo_image_surface_create(CAIRO_FORMAT_ARGB32, pixels, pixels);
    cairo_t *cr = cairo_create(surface);
    cairo_set_source_rgb(cr, 1.0, 1.0, 1.0);
    cairo_paint(cr);
    cairo_set_source_rgb(cr, 0.0, 0.0, 0.0);
    for (int y = 0; y < size; ++y) {
        for (int x = 0; x < size; ++x) {
            if (qrcodegen_getModule(code, x, y)) {
                cairo_rectangle(cr, (x + quiet) * scale, (y + quiet) * scale, scale, scale);
            }
        }
    }
    cairo_fill(cr);
    cairo_status_t status = cairo_surface_write_to_png(surface, path);
    cairo_destroy(cr);
    cairo_surface_destroy(surface);
    return status == CAIRO_STATUS_SUCCESS;
}
