"""CalcSuite conformance flow for the current 3.0.3 build.

Project code lives here, not in AndroidForge core. The flow exercises the real
Settings updater, which proves this is the current app rather than only a
generic home-screen smoke.
"""

import json
import os
import subprocess
import sys

ANDROIDFORGE = os.environ.get("ANDROIDFORGE_EXE", "androidforge")
SESSION = os.environ["ANDROIDFORGE_SESSION"]
RUN_ID = os.environ.get("ANDROIDFORGE_RUN_ID", "")
ARTIFACTS = os.environ.get("ANDROIDFORGE_ARTIFACTS", "")


def forge(*args):
    command = [ANDROIDFORGE, *args, "--json"]
    if SESSION:
        command.extend(["--session", SESSION])
    if RUN_ID:
        command.extend(["--run", RUN_ID])
    result = subprocess.run(
        command,
        text=True,
        encoding="utf-8",
        errors="replace",
        capture_output=True,
    )
    print(result.stdout, end="")
    print(result.stderr, end="", file=sys.stderr)
    if result.returncode != 0:
        raise SystemExit(result.returncode)
    return json.loads(result.stdout)


def main():
    forge("app", "launch", "com.calcsuite")
    forge("ui", "assert", "--text", "Calc Suite", "--timeout", "20")
    forge("ui", "tap", "--content-desc", "Settings")
    forge("ui", "scroll-to", "--text", "Check for updates")
    forge("ui", "assert", "--text", "Check for updates")
    forge("ui", "tap", "--text", "Check")
    forge("ui", "wait", "--contains", "You have the newest version", "--timeout", "30")
    forge("ui", "assert", "--contains", "You have the newest version (3.0.3).")
    forge("capture", "screenshot", "--label", "calcsuite-updater")
    print(json.dumps({"artifacts": ARTIFACTS, "status": "passed", "version": "3.0.3"}))


if __name__ == "__main__":
    main()
