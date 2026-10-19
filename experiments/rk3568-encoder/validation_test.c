/* SPDX-License-Identifier: GPL-2.0 */
/* Exercise the exact pure helper compiled into the driver, without a device. */
#include <assert.h>
#include <stdio.h>
#include "hantro_h264_enc_validate.h"

int main(void)
{
    struct v4l2_ctrl_h264_encode_params p = { .slice_type = V4L2_H264_SLICE_TYPE_I };
    struct v4l2_ctrl_h264_encode_rc r = { .qp = 26, .qp_min = 0, .qp_max = 51 };
    unsigned int qp;
    for (qp = 0; qp <= 51; qp++) {
        r.qp = qp;
        assert(hantro_h264_enc_validate(&p, &r, 1920, 1088) == 0);
    }
    r.qp = 52;
    assert(hantro_h264_enc_validate(&p, &r, 1920, 1088) == -EINVAL);
    r.qp = ~0U;
    assert(hantro_h264_enc_validate(&p, &r, 1920, 1088) == -EINVAL);
    r.qp = 26;
    assert(hantro_h264_enc_validate(NULL, &r, 1920, 1088) == -EINVAL);
    assert(hantro_h264_enc_validate(&p, NULL, 1920, 1088) == -EINVAL);
    assert(hantro_h264_enc_validate(&p, &r, 0, 1088) == -EINVAL);
    assert(hantro_h264_enc_validate(&p, &r, 3840, 2160) == -EINVAL);
    assert(hantro_h264_enc_validate(&p, &r, 1920, 1080) == -EINVAL);
    r.qp_min = 27;
    assert(hantro_h264_enc_validate(&p, &r, 640, 480) == -EINVAL);
    r.qp_min = 0; r.qp_max = 25;
    assert(hantro_h264_enc_validate(&p, &r, 640, 480) == -EINVAL);
    r.qp_max = 52;
    assert(hantro_h264_enc_validate(&p, &r, 640, 480) == -EINVAL);
    r.qp_max = 51; p.cabac_init_idc = 3;
    assert(hantro_h264_enc_validate(&p, &r, 640, 480) == -EINVAL);
    p.cabac_init_idc = 2; p.slice_type = V4L2_H264_SLICE_TYPE_B;
    assert(hantro_h264_enc_validate(&p, &r, 640, 480) == -EINVAL);
    p.slice_type = V4L2_H264_SLICE_TYPE_P; p.slice_size_mb_rows = -1;
    assert(hantro_h264_enc_validate(&p, &r, 640, 480) == -EINVAL);
    p.slice_size_mb_rows = 31;
    assert(hantro_h264_enc_validate(&p, &r, 640, 480) == -EINVAL);
    p.slice_size_mb_rows = 30;
    assert(hantro_h264_enc_validate(&p, &r, 640, 480) == 0);
    p.flags = 1U << 31;
    assert(hantro_h264_enc_validate(&p, &r, 640, 480) == -EINVAL);
    p.flags = 0; p.pic_init_qp_minus26 = 26;
    assert(hantro_h264_enc_validate(&p, &r, 640, 480) == -EINVAL);
    p.pic_init_qp_minus26 = -27;
    assert(hantro_h264_enc_validate(&p, &r, 640, 480) == -EINVAL);
    p.pic_init_qp_minus26 = 0; p.chroma_qp_index_offset = 13;
    assert(hantro_h264_enc_validate(&p, &r, 640, 480) == -EINVAL);
    p.chroma_qp_index_offset = 0; p.slice_alpha_c0_offset_div2 = -7;
    assert(hantro_h264_enc_validate(&p, &r, 640, 480) == -EINVAL);
    p.slice_alpha_c0_offset_div2 = 0; p.slice_beta_offset_div2 = 7;
    assert(hantro_h264_enc_validate(&p, &r, 640, 480) == -EINVAL);
    p.slice_beta_offset_div2 = 0; p.disable_deblocking_filter_idc = 3;
    assert(hantro_h264_enc_validate(&p, &r, 640, 480) == -EINVAL);
    puts("52 valid QPs and invalid boundary/control checks passed; no device access.");
    return 0;
}
