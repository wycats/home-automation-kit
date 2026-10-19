import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import rootfs_manifest
import validate_bundle


class KernelPayloadTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name).resolve()
        for path in ["boot/Image", "boot/odroid-m1.dtb", "uapi/include/linux/videodev2.h",
                     "rootfs/lib/modules/7.2.11/modules.dep",
                     "rootfs/lib/modules/7.2.11/kernel/rockchip-vdec.ko"]:
            dest = self.root / path
            dest.parent.mkdir(parents=True, exist_ok=True)
            dest.write_bytes(b"fixture")
        (self.root / "kernelrelease").write_text("7.2.11\n")

    def finalize(self, version="7.2.11 SMP"):
        with patch.object(validate_bundle.subprocess, "check_output", return_value=version):
            validate_bundle.kernel(self.root)

    def test_roundtrip_and_tamper(self):
        self.finalize()
        validate_bundle.verify(self.root)
        (self.root / "boot/Image").write_bytes(b"tampered")
        with self.assertRaisesRegex(ValueError, "mismatch"):
            validate_bundle.verify(self.root)

    def test_unlisted_file_rejected(self):
        self.finalize()
        (self.root / "extra").write_bytes(b"unlisted")
        with self.assertRaisesRegex(ValueError, "mismatch"):
            validate_bundle.verify(self.root)

    def test_private_key_rejected(self):
        (self.root / "signing_key.pem").write_bytes(b"synthetic")
        with self.assertRaisesRegex(ValueError, "Private-key"):
            self.finalize()

    def test_release_mismatch(self):
        with self.assertRaisesRegex(ValueError, "Mismatched"):
            self.finalize("7.2.10 SMP")

    def test_missing_decoder(self):
        (self.root / "rootfs/lib/modules/7.2.11/kernel/rockchip-vdec.ko").unlink()
        with self.assertRaisesRegex(ValueError, "Missing decoder"):
            self.finalize()

    def test_unexpected_payload_file(self):
        (self.root / "build.log").write_text("unreviewed")
        with self.assertRaisesRegex(ValueError, "allowlist"):
            self.finalize()

    def test_symlink_directory_rejected(self):
        try:
            (self.root / "source").symlink_to(self.root / "boot", target_is_directory=True)
        except OSError:
            self.skipTest("Host cannot create unprivileged symlinks")
        with self.assertRaisesRegex(ValueError, "symlink"):
            self.finalize()


class RootfsTests(unittest.TestCase):
    def test_mount_configuration_rejected(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d).resolve()
            (root / "etc").mkdir()
            (root / "etc/fstab").write_text("synthetic")
            with self.assertRaisesRegex(ValueError, "configuration"):
                rootfs_manifest.inspect(root)

    def test_dangling_link_rejected(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d).resolve()
            try:
                (root / "link").symlink_to("missing")
            except OSError:
                self.skipTest("Host cannot create unprivileged symlinks")
            with self.assertRaisesRegex(ValueError, "dangling"):
                rootfs_manifest.inspect(root)

    def test_manifest_records_content_and_console(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d).resolve()
            (root / "init").write_text("synthetic")
            data = rootfs_manifest.inspect(root)
            self.assertEqual(data["files"][0]["path"], "init")
            self.assertEqual(data["additional_cpio_nodes"][0]["major"], 5)
            json.dumps(data)


if __name__ == "__main__":
    unittest.main()
