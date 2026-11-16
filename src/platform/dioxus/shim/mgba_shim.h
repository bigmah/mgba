/* Copyright (c) 2026 the mGBA Dioxus frontend contributors
 *
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */
#ifndef MGBA_DIOXUS_SHIM_H
#define MGBA_DIOXUS_SHIM_H

/* A deliberately tiny, ABI-stable C surface over libmgba's mCore vtable.
 * The Rust frontend only ever talks to these functions, so it never has to
 * mirror mGBA's (feature-flag dependent) struct layouts. */

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct MShim MShim;

/* Loads a ROM. `save_path` may be NULL for no battery save. Returns NULL on failure. */
MShim* mshim_open(const char* rom_path, const char* save_path);
void mshim_close(MShim*);

void mshim_reset(MShim*);
void mshim_run_frame(MShim*);
void mshim_set_keys(MShim*, uint32_t keys);

/* Copies the current frame into `rgba` as tightly packed RGBA8888 (alpha = 255).
 * `rgba` must hold at least w*h*4 bytes where w/h come from mshim_video_size. */
void mshim_video_size(MShim*, unsigned* width, unsigned* height);
void mshim_copy_frame_rgba(MShim*, uint8_t* rgba);

/* Reads up to `max_frames` interleaved stereo frames. Returns frames written. */
size_t mshim_read_audio(MShim*, int16_t* out, size_t max_frames);
unsigned mshim_sample_rate(MShim*);
/* Native refresh rate (~59.7275 Hz for GBA). */
double mshim_fps(MShim*);

/* mGBA-compatible savestates (PNG-wrapped, with an embedded screenshot). */
bool mshim_save_state(MShim*, const char* path);
bool mshim_load_state(MShim*, const char* path);
bool mshim_screenshot(MShim*, const char* path);

/* Returns 0 for GBA, 1 for GB, -1 otherwise (mirrors enum mPlatform). */
int mshim_platform(MShim*);

#ifdef __cplusplus
}
#endif

#endif
