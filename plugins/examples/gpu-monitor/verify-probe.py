"""Execute the exact packaged probe in a POSIX shell with controlled NVIDIA fixtures.

Windows: python plugins/examples/gpu-monitor/verify-probe.py --wsl Ubuntu
Linux:   python3 plugins/examples/gpu-monitor/verify-probe.py
"""
import argparse
import json
import subprocess
import zipfile
from pathlib import Path

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--wsl", metavar="DISTRO", help="Use an existing WSL distribution on Windows")
parser.add_argument("--package", type=Path, default=Path(__file__).resolve().parents[3] / "temp/plugins/gpu-monitor.nyap")
args = parser.parse_args()
shell = ["wsl", "-d", args.wsl, "--exec", "/bin/sh"] if args.wsl else ["/bin/sh"]
with zipfile.ZipFile(args.package) as package:
    manifest = json.loads(package.read("manifest.json"))
    assert manifest["id"] == "nyaterm.gpu"
    probe = package.read("assets/probes/gpu.sh")

# Reproduce the old Windows packaging failure, then exercise the actual fixed package.
assert b"\r" not in probe, "Packaged POSIX script contains Windows CRLF line endings"
assert not probe.startswith(b"\xef\xbb\xbf"), "Packaged script contains a UTF-8 BOM"
assert probe.endswith(b"\n")
def execute(script, command):
    result = subprocess.run(shell + ["-c", command], input=script, capture_output=True, timeout=20)
    return result.returncode, result.stdout, result.stderr

unavailable_command = "PATH=/nonexistent /bin/sh -s"
status, _, _ = execute(probe.replace(b"\n", b"\r\n"), unavailable_command)
assert status != 0, "CRLF regression did not reproduce in the Linux shell"
status, stdout, _ = execute(probe, unavailable_command)
assert status == 0 and stdout == b"GPU_AVAILABLE\t0\n", (status, stdout)

fixture_command = r'''set -eu
fixture=$(mktemp -d)
trap 'rm -rf "$fixture"' EXIT HUP INT TERM
cat > "$fixture/nvidia-smi" <<'NYATERM_NVIDIA_FIXTURE'
#!/bin/sh
case "$*" in
  --query-gpu=*)
    printf '%s\n' '0, GPU-a, NVIDIA RTX 4090, 550.54, 43, 77, 31, 24564, 12345, 12219, 180.5, 450, 46, P2'
    ;;
  --query-compute-apps=*)
    printf '%s\n' 'GPU-a, 4242, 2048, python'
    ;;
  *)
    printf '%s\n' '| NVIDIA-SMI 550.54   Driver Version: 550.54   CUDA Version: 12.4 |'
    ;;
esac
NYATERM_NVIDIA_FIXTURE
chmod +x "$fixture/nvidia-smi"
PATH="$fixture:/usr/bin:/bin" /bin/sh -s
'''
status, stdout, _ = execute(probe, fixture_command)
assert status == 0, f"GPU fixture probe exited with {status}"
for required in [b"GPU_AVAILABLE\t1\n", b"GPU_CUDA_VERSION\t12.4\n", b"GPU_CSV_BEGIN\n", b"NVIDIA RTX 4090", b"GPU_CSV_END\n", b"GPU_PROCESS_CSV_BEGIN\n", b"GPU-a, 4242, 2048, python", b"GPU_PROCESS_CSV_END\n"]:
    assert required in stdout, f"Probe did not produce {required!r}"
print(f"GPU probe {manifest['version']} passed: exact package LF bytes, CRLF regression, unavailable and NVIDIA fixtures in a Linux/POSIX shell.")
