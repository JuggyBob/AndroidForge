"""DiscountHunter smoke conformance.

Project code lives here, not in AndroidForge core. The flow uses the real
1.1.x debug build: permission state, semantic navigation, the app's own
background-check receiver, and bounded evidence.

It accepts either a fresh install (intro pages and the notification
permission request) or an already-used install. No fixed sleep is required.
"""

import json
import os
import subprocess
import sys

ANDROIDFORGE = os.environ.get("ANDROIDFORGE_EXE", "androidforge")
SESSION = os.environ["ANDROIDFORGE_SESSION"]
RUN_ID = os.environ.get("ANDROIDFORGE_RUN_ID", "")
ARTIFACTS = os.environ.get("ANDROIDFORGE_ARTIFACTS", "")
PACKAGE = "com.discounthunter.android.debug"

checks = []
optional = []


def run(*args, required=True):
    command = [ANDROIDFORGE, *args, "--json"]
    if SESSION:
        command.extend(["--session", SESSION])
    if RUN_ID:
        command.extend(["--run", RUN_ID])
    if not required:
        command.append("--no-failure-capture")
    result = subprocess.run(
        command,
        text=True,
        encoding="utf-8",
        errors="replace",
        capture_output=True,
    )
    if required and result.returncode != 0:
        print(result.stdout, end="")
        print(result.stderr, end="", file=sys.stderr)
        raise SystemExit(result.returncode)
    payload = None
    if result.stdout.strip():
        try:
            payload = json.loads(result.stdout)
        except json.JSONDecodeError:
            payload = {"raw_stdout": result.stdout}
    return result.returncode, payload


def check(name, passed, detail):
    checks.append({"name": name, "passed": bool(passed), "detail": detail})
    if not passed:
        raise SystemExit(10)


def find(*args):
    """`ui find` is a search: exit 0 with an empty match list means absent."""
    code, payload = run("ui", "find", *args, required=False)
    if code != 0 or not payload:
        return []
    return payload.get("result", {}).get("matches", [])


def dismiss_first_run():
    """Walk the real intro and notification request if this install is new."""
    if find("--text", "Deals"):
        return
    if not find("--text", "Start hunting"):
        for _ in range(6):
            code, _ = run("ui", "tap", "--text", "Next", required=False)
            if code != 0:
                break
        run("ui", "tap", "--text", "Start hunting", required=False)
        run("ui", "tap", "--text", "Not now", required=False)


def main():
    launch = run("app", "launch", PACKAGE)[1]
    check(
        "app foreground",
        launch["result"].get("foreground") is True,
        launch["result"].get("foreground_app"),
    )

    dismiss_first_run()
    run("ui", "wait", "--text", "Deals", "--timeout", "20")
    dismiss_first_run()

    run("ui", "tap", "--text", "Settings")
    run("ui", "scroll-to", "--text", "Deal alerts")
    alerts = run("ui", "assert", "--text", "Deal alerts")[1]
    check("deal alerts section", alerts["result"].get("matched") is True, alerts["result"])

    permission = run(
        "permissions", "check", "android.permission.POST_NOTIFICATIONS"
    )[1]
    state = permission["result"]["state"]
    check(
        "notification permission",
        state.get("granted") is True,
        {"permission": state.get("permission"), "source": state.get("source")},
    )

    hooked = run("hook", "run", "check-deals")[1]
    check(
        "background check hook",
        hooked["result"].get("success") is True,
        {
            "kind": hooked["result"].get("kind"),
            "command": hooked["result"].get("command"),
        },
    )

    run("ui", "wait", "--contains", "Last check", "--timeout", "20")
    last_check = run("ui", "assert", "--contains", "Last check")[1]
    check("check result visible", last_check["result"].get("matched") is True, last_check["result"])

    screenshot = run("capture", "screenshot", "--label", "discount-hunter-alerts")[1]
    artifacts = [
        item["path"] for item in screenshot.get("artifacts", []) if item["kind"] == "screenshot"
    ]
    check("screenshot evidence", bool(artifacts), artifacts)

    run("diagnostics", "collect", "--package", PACKAGE)

    code, channels = run("notifications", "channels", "--package", PACKAGE, required=False)
    if code == 0 and channels:
        optional.append(
            {
                "name": "notification channels",
                "best_effort": channels["result"].get("best_effort"),
                "channels": channels["result"].get("channels"),
            }
        )

    print(
        json.dumps(
            {
                "status": "success",
                "package": PACKAGE,
                "artifacts": ARTIFACTS,
                "checks": checks,
                "optional": optional,
            }
        )
    )


if __name__ == "__main__":
    main()
