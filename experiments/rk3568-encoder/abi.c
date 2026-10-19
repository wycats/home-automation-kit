/* SPDX-License-Identifier: MIT */
/* This only inspects types; it does not open a device. */
#include <stddef.h>
#include <stdio.h>
#include <linux/videodev2.h>
#include <linux/v4l2-controls.h>

_Static_assert(V4L2_CTRL_TYPE_H264_ENCODE_PARAMS == 0x0206, "legacy params ABI");
_Static_assert(V4L2_CTRL_TYPE_H264_ENCODE_RC == 0x0207, "legacy RC ABI");
_Static_assert(V4L2_CTRL_TYPE_H264_ENCODE_FEEDBACK == 0x0208, "legacy feedback ABI");
_Static_assert(sizeof(((struct v4l2_ctrl_h264_encode_rc *)0)->cp_target) / sizeof(__u32) == 10, "checkpoint count");
_Static_assert(offsetof(struct v4l2_ctrl_h264_encode_params, reference_ts) % 8 == 0, "reference alignment");
int main(void)
{
    printf("{\"params_bytes\":%zu,\"rc_bytes\":%zu,\"feedback_bytes\":%zu,\"hardware_tested\":false}\n",
           sizeof(struct v4l2_ctrl_h264_encode_params),
           sizeof(struct v4l2_ctrl_h264_encode_rc),
           sizeof(struct v4l2_ctrl_h264_encode_feedback));
    return 0;
}
