/* SPDX-License-Identifier: GPL-2.0 */
#include <assert.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

struct variant { int num_clocks; };
struct hantro_dev {
	void *dev, *resets, *clocks, *m2m_dev;
	int mdev, v4l2_dev;
	struct variant *variant;
};
struct platform_device { struct hantro_dev *data; const char *name; };
static int autosuspend_calls, disable_calls, reset_calls, reset_asserted;
static int reset_error, clocks, v4l2, m2m, media, enc, dec, unregistered;
static int pending_pm;

/* Model the synchronous callback that exposed the upstream crash. */
static void pm_runtime_dont_use_autosuspend(void *dev)
{
	assert(!reset_asserted);
	assert(disable_calls == 0);
	assert(++autosuspend_calls == 1);
	pending_pm = 1;
}
static void pm_runtime_disable(void *dev)
{
	assert(!reset_asserted);
	assert(autosuspend_calls == 1 && pending_pm);
	assert(++disable_calls == 1);
	pending_pm = 0;
}
static int reset_control_assert(void *resets)
{
	assert(disable_calls == 1 && !pending_pm);
	assert(++reset_calls == 1);
	reset_asserted = resets && !reset_error;
	return reset_error;
}
static void clk_bulk_unprepare(int n, void *c) { assert(clocks-- == 1); }
static void v4l2_device_unregister(void *p) { assert(v4l2-- == 1); }
static void v4l2_m2m_put(void *p) { assert(m2m-- == 1); }
static void media_device_cleanup(void *p) { assert(media-- == 1); }
static void hantro_remove_enc_func(struct hantro_dev *v) { assert(enc-- == 1); }
static void hantro_remove_dec_func(struct hantro_dev *v) { assert(dec-- == 1); }
static void media_device_unregister(void *p) { assert(++unregistered == 1); }
static struct hantro_dev *platform_get_drvdata(struct platform_device *p) { return p->data; }
#define v4l2_info(...) ((void)0)
#include "teardown_subject.inc"

static void initialize(int stage, int mode)
{
	autosuspend_calls = disable_calls = reset_calls = reset_asserted = pending_pm = 0;
	unregistered = 0;
	reset_error = mode == 1 ? -5 : 0;
	clocks = stage >= 2;
	v4l2 = stage >= 3;
	m2m = media = stage >= 4;
	enc = stage >= 5;
	dec = stage >= 6;
}

int main(void)
{
	struct variant variant = { .num_clocks = 2 };
	struct hantro_dev vpu = { .variant = &variant };
	struct platform_device pdev = { .data = &vpu, .name = "synthetic" };
	int cases = 0;

	/* Normal reset, failed reset, and an absent optional reset handle. */
	for (int mode = 0; mode < 3; mode++) {
		vpu.resets = mode == 2 ? NULL : &vpu;
		for (int stage = 0; stage < 7; stage++) {
			initialize(stage, mode);
			assert(probe_failure(&vpu, stage, -123) == -123);
			assert(autosuspend_calls == 1 && disable_calls == 1);
			assert(reset_calls == (stage != 0));
			assert(!clocks && !v4l2 && !m2m && !media && !enc && !dec);
			assert(!unregistered);
			cases++;
		}
		initialize(7, mode);
		hantro_remove(&pdev);
		assert(autosuspend_calls == 1 && disable_calls == 1 && reset_calls == 1);
		assert(!clocks && !v4l2 && !m2m && !media && !enc && !dec);
		assert(unregistered == 1);
		cases++;
	}
	printf("%d extracted probe/remove cases passed; PM callbacks precede reset. No DMA claim.\n", cases);
	return 0;
}
