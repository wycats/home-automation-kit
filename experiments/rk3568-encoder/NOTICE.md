# Attribution and redistribution

The kernel patch derives from Paul Kocialkowski's Bootlin Hantro H.264 prototype
and Michael Grzeschik's Pengutronix RK3568 adaptation. The four original commit
identities are pinned in `manifest.json`. The API commit is authored and signed
off by Paul Kocialkowski; the driver commit is authored by Michael Grzeschik and
retains Paul's sign-off. The RK3568 enablement and follow-up are authored by
Michael Grzeschik. This rebase is a local experiment, not an upstream submission.
Existing source copyright and SPDX notices remain intact. Derived kernel code
is GPL-2.0; see `../rk3568-kernel/LICENSES/GPL-2.0.txt`.

The PM-before-reset teardown correction is by Tharit Tangkijwanichakul, from
media.git commit `8e26d4c20ed218db6c3123480a4a4e2545680566`. Its upstream
review/sign-off provenance and unchanged scope are described in RESET-ORDER.md.

Original build orchestration and ABI probe are MIT. This entire directory is
excluded from the Rust crate package, so its patches are not relabeled MIT.

The separate Bootlin client is fetched at its pinned commit and compiled only.
No license declaration was found in that tree at inspection. Neither its source
nor its executable is uploaded in this experiment's artifacts. Clarify its
redistribution terms or implement a separately licensed client before packaging
it in a distributable root filesystem.
