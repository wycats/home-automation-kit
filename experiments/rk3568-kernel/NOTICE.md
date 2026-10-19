# Source and licensing

This directory has mixed licensing and is excluded from the MIT Rust crate's
package. The repository's MIT license applies to original orchestration scripts;
it does not relicense the kernel patches or imported configuration files.

- Linux driver patches and derived C test code retain their kernel-source
  attribution and GPL-2.0 terms. The complete license is in
  [LICENSES/GPL-2.0.txt](LICENSES/GPL-2.0.txt).
- The unmodified `config/` inputs come from Home Assistant Operating System commit
  `b22f929d3bc8895f5cf84d13d71a24714b59eed4` under Apache-2.0. Each exact source URL
  and content hash is in `manifest.json`. The complete license is in
  [LICENSES/Apache-2.0.txt](LICENSES/Apache-2.0.txt).
- `rk3568-01.patch` through `rk3568-03.patch` are
  [Christian Hewitt's VDPU346 v3 series](https://patchew.org/linux/20260110053727.2295260-1-christianshewitt%40gmail.com/);
  the original descriptions, sign-offs, acknowledgements and test credits are
  preserved. They are imported experiments, not changes authored by this repo.
- The canonical RCB resize and RPS-bounds patches are copied unchanged from
  [gregordinary/patches at ac458850](https://github.com/gregordinary/patches/tree/ac458850f0c4f522b47a12f73c9ce101867d5369/media-accel/kernel),
  retaining Detlev Casanova's and gregordinary's attribution respectively.
- `lab-06`, `lab-07` and `lab-08` are local experimental followups, not submitted
  upstream. Their original source attribution remains in the patched files.
  The buffer envelope's independent formula reference is
  [Rockchip MPP at 20959ec3](https://github.com/rockchip-linux/mpp/tree/20959ec3dabb313b8c33ca0b7b09c06ffd2a9caa).

No household configuration, inventory, addresses, footage or credentials are
part of this experiment.
