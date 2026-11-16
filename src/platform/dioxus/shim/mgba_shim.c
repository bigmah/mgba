/* Copyright (c) 2026 the mGBA Dioxus frontend contributors
 *
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */
#include "mgba_shim.h"

#include <mgba/flags.h>
#include <mgba/core/config.h>
#include <mgba/core/core.h>
#include <mgba/core/log.h>
#include <mgba/core/serialize.h>
#include <mgba-util/audio-buffer.h>
#include <mgba-util/image.h>
#include <mgba-util/vfs.h>

#include <fcntl.h>
#include <stdarg.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#define SHIM_MAX_WIDTH 256
#define SHIM_MAX_HEIGHT 224
#define SHIM_AUDIO_SAMPLES 2048

struct MShim {
	struct mCore* core;
	mColor* video;
};

/* mGBA logs a *lot* at INFO/DEBUG. Only surface real problems unless MGBA_LOG is set. */
static void _shimLog(struct mLogger* logger, int category, enum mLogLevel level, const char* format, va_list args) {
	(void) logger;
	static int verbose = -1;
	if (verbose < 0) {
		const char* env = getenv("MGBA_LOG");
		verbose = env && env[0] && env[0] != '0';
	}
	if (!verbose && !(level & (mLOG_FATAL | mLOG_ERROR))) {
		return;
	}
	fprintf(stderr, "[mgba] %s: ", mLogCategoryName(category));
	vfprintf(stderr, format, args);
	fputc('\n', stderr);
}

static struct mLogger _logger = { .log = _shimLog };

MShim* mshim_open(const char* romPath, const char* savePath) {
	mLogSetDefaultLogger(&_logger);
	struct mCore* core = mCoreFind(romPath);
	if (!core) {
		return NULL;
	}
	if (!core->init(core)) {
		return NULL;
	}
	mCoreInitConfig(core, NULL);

	struct mCoreOptions opts = {
		.useBios = false,
		.skipBios = true,
		.volume = 0x100,
		.sampleRate = 48000,
		.audioBuffers = SHIM_AUDIO_SAMPLES,
	};
	mCoreConfigLoadDefaults(&core->config, &opts);
	mCoreConfigSetDefaultValue(&core->config, "idleOptimization", "detect");
	mCoreLoadForeignConfig(core, &core->config);

	MShim* shim = calloc(1, sizeof(*shim));
	shim->core = core;
	shim->video = calloc(SHIM_MAX_WIDTH * SHIM_MAX_HEIGHT, sizeof(mColor));
	core->setVideoBuffer(core, shim->video, SHIM_MAX_WIDTH);
	core->setAudioBufferSize(core, SHIM_AUDIO_SAMPLES);

	if (!mCoreLoadFile(core, romPath)) {
		mshim_close(shim);
		return NULL;
	}
	if (savePath && savePath[0]) {
		struct VFile* save = VFileOpen(savePath, O_CREAT | O_RDWR);
		if (save && !core->loadSave(core, save)) {
			save->close(save);
		}
	}
	core->reset(core);
	return shim;
}

void mshim_close(MShim* shim) {
	if (!shim) {
		return;
	}
	if (shim->core) {
		mCoreConfigDeinit(&shim->core->config);
		/* deinit unloads the ROM and flushes/unmaps the battery save. */
		shim->core->deinit(shim->core);
	}
	free(shim->video);
	free(shim);
}

void mshim_reset(MShim* shim) {
	shim->core->reset(shim->core);
}

void mshim_run_frame(MShim* shim) {
	shim->core->runFrame(shim->core);
}

void mshim_set_keys(MShim* shim, uint32_t keys) {
	shim->core->setKeys(shim->core, keys);
}

void mshim_video_size(MShim* shim, unsigned* width, unsigned* height) {
	shim->core->currentVideoSize(shim->core, width, height);
}

void mshim_copy_frame_rgba(MShim* shim, uint8_t* rgba) {
	unsigned width, height;
	shim->core->currentVideoSize(shim->core, &width, &height);
	for (unsigned y = 0; y < height; ++y) {
		const mColor* row = &shim->video[y * SHIM_MAX_WIDTH];
		for (unsigned x = 0; x < width; ++x) {
			mColor c = row[x];
			uint8_t* out = &rgba[(y * width + x) * 4];
#ifdef COLOR_16_BIT
			/* Not used by default builds, but keep it correct anyway (RGB555/BGR555). */
			out[0] = ((c >> 0) & 0x1F) << 3;
			out[1] = ((c >> 5) & 0x1F) << 3;
			out[2] = ((c >> 10) & 0x1F) << 3;
#else
			/* mGBA's 32-bit colour is XBGR8888: R in the low byte. */
			out[0] = c & 0xFF;
			out[1] = (c >> 8) & 0xFF;
			out[2] = (c >> 16) & 0xFF;
#endif
			out[3] = 0xFF;
		}
	}
}

size_t mshim_read_audio(MShim* shim, int16_t* out, size_t maxFrames) {
	struct mAudioBuffer* buffer = shim->core->getAudioBuffer(shim->core);
	if (!buffer) {
		return 0;
	}
	size_t available = mAudioBufferAvailable(buffer);
	if (available > maxFrames) {
		available = maxFrames;
	}
	return mAudioBufferRead(buffer, out, available);
}

unsigned mshim_sample_rate(MShim* shim) {
	return shim->core->audioSampleRate(shim->core);
}

double mshim_fps(MShim* shim) {
	int32_t cycles = shim->core->frameCycles(shim->core);
	if (!cycles) {
		return 60.0;
	}
	return (double) shim->core->frequency(shim->core) / (double) cycles;
}

bool mshim_save_state(MShim* shim, const char* path) {
	struct VFile* vf = VFileOpen(path, O_CREAT | O_TRUNC | O_RDWR);
	if (!vf) {
		return false;
	}
	bool ok = mCoreSaveStateNamed(shim->core, vf, SAVESTATE_SCREENSHOT | SAVESTATE_SAVEDATA | SAVESTATE_RTC | SAVESTATE_METADATA);
	vf->close(vf);
	return ok;
}

bool mshim_load_state(MShim* shim, const char* path) {
	struct VFile* vf = VFileOpen(path, O_RDONLY);
	if (!vf) {
		return false;
	}
	/* Like mGBA's defaults: don't clobber the battery save from a state. */
	bool ok = mCoreLoadStateNamed(shim->core, vf, SAVESTATE_RTC);
	vf->close(vf);
	return ok;
}

bool mshim_screenshot(MShim* shim, const char* path) {
	struct VFile* vf = VFileOpen(path, O_CREAT | O_TRUNC | O_WRONLY);
	if (!vf) {
		return false;
	}
	bool ok = mCoreTakeScreenshotVF(shim->core, vf);
	vf->close(vf);
	return ok;
}

int mshim_platform(MShim* shim) {
	return (int) shim->core->platform(shim->core);
}
