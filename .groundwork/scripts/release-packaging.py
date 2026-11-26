#!/usr/bin/env python3
"""Exercise the real package helpers with owned executable fixtures; launch nothing."""
import hashlib
import json
import os
from pathlib import Path
import plistlib
import stat
import subprocess
import tempfile
import zipfile

ROOT = Path(__file__).resolve().parents[2]
EVIDENCE = Path(os.environ.get("GROUNDWORK_EVIDENCE_DIR", ROOT / ".groundwork/tmp/release-packaging")).resolve()
ASSET_URL = "https://github.com/darylcecile/karabiner/releases/download/v0.1.0/karabiner-service-macos-arm64"
EVIDENCE.mkdir(parents=True, exist_ok=True)


def run_helper(name, args, env):
    with (EVIDENCE / f"{name}.log").open("w") as log:
        subprocess.run(["bash", str(ROOT / f".github/scripts/{name}.sh"), *map(str, args)],
                       cwd=ROOT, env=env, stdout=log, stderr=subprocess.STDOUT, check=True)


def inspect_packages():
    env = {**os.environ, "KARABINER_SERVICE_ASSET_URL": ASSET_URL}
    with tempfile.TemporaryDirectory(prefix="karabiner-package-fixture-") as directory:
        work = Path(directory)
        client, service = work / "client", work / "service"
        client_bytes = b"#!/bin/sh\n# client package fixture; never executed\nexit 0\n"
        service_bytes = b"#!/bin/sh\n# service package fixture; never executed\nexit 0\n"
        for path, data in [(client, client_bytes), (service, service_bytes)]:
            path.write_bytes(data)
            path.chmod(0o755)
        run_helper("package-client", [client, "0.1.0", EVIDENCE], env)
        run_helper("package-service", [service, EVIDENCE], env)
        with zipfile.ZipFile(EVIDENCE / "Karabiner-macos-arm64.zip") as archive:
            info = plistlib.loads(archive.read("Karabiner.app/Contents/Info.plist"))
            executable = "Karabiner.app/Contents/MacOS/" + info["CFBundleExecutable"]
            assert info["CFBundlePackageType"] == "APPL", "Bundle is not an application"
            assert info["CFBundleVersion"] == "0.1.0", "Client version not preserved"
            assert info["KarabinerServiceAssetURL"] == ASSET_URL, "Exact tag URL not stamped"
            assert archive.read(executable) == client_bytes, "Client payload changed"
            mode = archive.getinfo(executable).external_attr >> 16
            assert mode & stat.S_IXUSR, "Archive lost the executable bit"
            for name in ["React-Flow-NOTICE.md", "Kubai-LICENSE.txt"]:
                assert b"MIT License" in archive.read(f"Karabiner.app/Contents/Resources/{name}"), "License notice missing"
        asset = EVIDENCE / "karabiner-service-macos-arm64"
        digest, filename = (EVIDENCE / "karabiner-service-macos-arm64.sha256").read_text().split()
        assert filename == asset.name, "Checksum names the wrong asset"
        assert asset.read_bytes() == service_bytes, "Service payload changed"
        assert digest == hashlib.sha256(service_bytes).hexdigest(), "Checksum does not match the service"
        assert asset.stat().st_mode & stat.S_IXUSR, "Packaged service is not executable"
        return {"bundle": "Karabiner.app", "version": info["CFBundleVersion"],
                "serviceAssetURL": ASSET_URL, "clientMode": oct(mode), "serviceSha256": digest,
                "fixture": "synthetic executable payloads; no app or service was launched"}


try:
    details = inspect_packages()
    (EVIDENCE / "package-inspection.json").write_text(json.dumps(details, indent=2) + "\n")
    result = {"observations": [{"id": "release-package-layout", "status": "passed",
              "observed": "Real helpers preserve executable bytes/mode in the app ZIP, valid plist/version/exact service URL, notices and raw service checksum",
              "evidence": ["package-inspection.json", "package-client.log", "package-service.log", "Karabiner-macos-arm64.zip", "karabiner-service-macos-arm64", "karabiner-service-macos-arm64.sha256"]}],
              "context": {"fixture": "synthetic-executable-packaging", "appLaunched": False, "externalWrites": False}}
    code = 0
except (OSError, ValueError, KeyError, AssertionError, subprocess.CalledProcessError, zipfile.BadZipFile) as error:
    (EVIDENCE / "package-error.txt").write_text(str(error) + "\n")
    result = {"observations": [{"id": "release-package-layout", "status": "failed",
              "observed": str(error) or type(error).__name__, "evidence": ["package-error.txt"]}]}
    code = 1
(EVIDENCE / "result.json").write_text(json.dumps(result, indent=2) + "\n")
print(result["observations"][0]["observed"])
raise SystemExit(code)
