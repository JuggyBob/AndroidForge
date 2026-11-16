"""IronPulse smoke conformance.

This is project code. It exercises the shipped 4.x app without adding an
IronPulse branch to AndroidForge:
  * first-run onboarding or an already-onboarded home screen
  * process, task and foreground liveness
  * the real widget intents handled by MainActivity
  * Settings navigation, a screenshot, a short screen recording and diagnostics

Compose test tags are internal to the app and are not exposed through
UIAutomator (`testTagsAsResourceId` is not set). The flow therefore uses visible
semantics and says so rather than pretending internal tags are selectable.
"""

import json
import os
import subprocess
import sys
import time

ANDROIDFORGE = os.environ.get("ANDROIDFORGE_EXE", "androidforge")
SESSION = os.environ["ANDROIDFORGE_SESSION"]
RUN_ID = os.environ.get("ANDROIDFORGE_RUN_ID", "")
ARTIFACTS = os.environ.get("ANDROIDFORGE_ARTIFACTS", "")
PACKAGE = "com.aistudio.fitness.ironpulse"

checks = []
notes = []


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
        print(json.dumps({"status": "failure", "package": PACKAGE, "checks": checks}))
        raise SystemExit(10)


def find(*args):
    """`ui find` is a search: exit 0 with an empty match list means absent."""
    code, payload = run("ui", "find", *args, required=False)
    if code != 0 or not payload:
        return []
    return payload.get("result", {}).get("matches", [])


def on_home():
    # The bottom navigation "Home" label also appears on Settings, and the
    # welcome sentence appears on the first-run intro. The streak heading is
    # unique to the Home content.
    return bool(find("--text", "CURRENT STREAK"))


def complete_onboarding_if_needed():
    if on_home():
        return
    run("ui", "tap", "--text", "Home", required=False)
    if on_home():
        return
    run("ui", "tap", "--text", "Skip", required=False)
    code, _ = run("ui", "wait", "--text", "Start training", "--timeout", "10", required=False)
    if code != 0:
        return
    values = ["AndroidForge", "30", "180", "80", "75"]
    for index, value in enumerate(values):
        run(
            "ui",
            "text",
            value,
            "--class",
            "android.widget.EditText",
            "--index",
            str(index),
            "--clear",
            "--hide-keyboard",
        )
    run("ui", "tap", "--text", "Start training")


def dismiss_update_dialog():
    """Dismiss a debug-build update prompt so it cannot cover Home."""
    if not find("--text", "Update available") and not find("--contains", "is available"):
        return
    run("ui", "tap", "--text", "Do not show this again", required=False)
    run("ui", "tap", "--text", "Close", required=False)
    run("ui", "wait", "--text", "CURRENT STREAK", "--timeout", "10", required=False)


def main():
    launch = run("app", "launch", PACKAGE)[1]
    check(
        "app foreground",
        launch["result"].get("foreground") is True,
        launch["result"].get("foreground_app"),
    )

    complete_onboarding_if_needed()
    dismiss_update_dialog()
    run("ui", "tap", "--text", "Home", required=False)
    dismiss_update_dialog()
    run("ui", "wait", "--text", "CURRENT STREAK", "--timeout", "20")
    home = run("ui", "assert", "--text", "CURRENT STREAK")[1]
    check("home surface", home["result"].get("matched") is True, home["result"])

    status = run(
        "app",
        "status",
        PACKAGE,
        "--expect",
        "foreground",
        "--assert-task-exists",
        "--assert-process-stable",
        "--stable-for",
        "2s",
    )[1]
    states = status["result"]["states"]
    check(
        "foreground process and task",
        states.get("foreground") is True
        and states.get("process_alive") is True
        and states.get("task_exists") is True,
        {
            "foreground": states.get("foreground"),
            "process": states.get("process"),
            "task_exists": states.get("task_exists"),
            "resumed": states.get("resumed"),
        },
    )

    streak = run("hook", "run", "focus-streak")[1]
    check("streak widget intent", streak["result"].get("success") is True, streak["result"].get("command"))

    profile = run("hook", "run", "open-profile")[1]
    check("profile widget intent", profile["result"].get("success") is True, profile["result"].get("command"))
    run("ui", "wait", "--text", "Settings", "--timeout", "20")
    settings = run("ui", "assert", "--contains", "Your account, appearance")[1]
    check("settings surface", settings["result"].get("matched") is True, settings["result"])

    screenshot = run("capture", "screenshot", "--label", "ironpulse-settings")[1]
    screenshot_paths = [
        item["path"] for item in screenshot.get("artifacts", []) if item["kind"] == "screenshot"
    ]
    check("screenshot evidence", bool(screenshot_paths), screenshot_paths)

    run("capture", "record", "start", "--time-limit", "10")
    time.sleep(2)
    recording = run("capture", "record", "stop")[1]
    recording_path = recording["result"].get("artifact", {}).get("path")
    check("screen recording", bool(recording_path), recording_path)

    run("diagnostics", "app", PACKAGE)
    notes.append(
        "Compose testTags are internal to the app; UIAutomator sees visible text only. "
        "Use instrumentation for tag-level assertions."
    )

    print(
        json.dumps(
            {
                "status": "success",
                "package": PACKAGE,
                "artifacts": ARTIFACTS,
                "checks": checks,
                "notes": notes,
            }
        )
    )


if __name__ == "__main__":
    main()
