#!/usr/bin/env python3
"""Execute CLI operations against a throwaway Linux Secret Service session.

Rust owns the full argv sequence, operator permission transitions and contract
assertions. This fixture owns the isolated bus/daemon lifetime, restores index
parent permissions on failure and captures output plus on-disk evidence.
The caller supplies HOME/XDG roots and retains LLVM_PROFILE_FILE for children.
"""

import json
import os
from pathlib import Path
import subprocess
import sys
import time


def run_session(home, commands):
    runtime = home / "runtime"
    runtime.mkdir(mode=0o700)
    control = runtime / "keyring"
    control.mkdir(mode=0o700)
    os.environ["XDG_RUNTIME_DIR"] = str(runtime)
    index = Path(os.environ["XDG_CONFIG_HOME"]) / "osp/secrets.index.toml"
    native_store = Path(os.environ["XDG_DATA_HOME"]) / "keyrings/login.keyring"
    with (home / "keyring-daemon.log").open("w+") as log:
        daemon = subprocess.Popen(
            [
                "gnome-keyring-daemon",
                "--foreground",
                "--unlock",
                "--components=secrets",
                "--control-directory",
                str(control),
            ],
            stdin=subprocess.PIPE,
            stdout=log,
            stderr=log,
            text=True,
        )
        try:
            daemon.stdin.write("throwaway-keyring-password\n")
            daemon.stdin.close()
            deadline = time.monotonic() + 20
            while True:
                owner = subprocess.run(
                    [
                        "gdbus",
                        "call",
                        "--session",
                        "--dest",
                        "org.freedesktop.DBus",
                        "--object-path",
                        "/org/freedesktop/DBus",
                        "--method",
                        "org.freedesktop.DBus.NameHasOwner",
                        "org.freedesktop.secrets",
                    ],
                    text=True,
                    capture_output=True,
                    timeout=3,
                )
                if owner.returncode == 0 and "true" in owner.stdout:
                    unlocked = subprocess.run(
                        [
                            "gdbus",
                            "call",
                            "--session",
                            "--dest",
                            "org.freedesktop.secrets",
                            "--object-path",
                            "/org/freedesktop/secrets/collection/login",
                            "--method",
                            "org.freedesktop.DBus.Properties.Get",
                            "org.freedesktop.Secret.Collection",
                            "Locked",
                        ],
                        text=True,
                        capture_output=True,
                        timeout=3,
                    )
                    if unlocked.returncode == 0 and "false" in unlocked.stdout:
                        break
                if daemon.poll() is not None or time.monotonic() >= deadline:
                    log.seek(0)
                    raise RuntimeError(
                        "isolated Secret Service did not unlock:\n" + log.read()
                    )
                time.sleep(0.05)
            results = []
            for args in commands:
                output = subprocess.run(
                    args, text=True, capture_output=True, timeout=20
                )
                results.append(
                    {
                        "exit_code": output.returncode,
                        "stdout": output.stdout,
                        "stderr": output.stderr,
                        "index": index.read_text() if index.exists() else None,
                        "mode": index.stat().st_mode & 0o777
                        if index.exists()
                        else None,
                        "native_store": native_store.is_file(),
                        "index_parent_mode": index.parent.stat().st_mode & 0o777,
                    }
                )
            print(json.dumps(results))
        finally:
            daemon.terminate()
            try:
                daemon.wait(timeout=3)
            except subprocess.TimeoutExpired:
                daemon.kill()
                daemon.wait(timeout=3)


def main():
    if sys.argv[1] == "--session":
        run_session(Path(sys.argv[2]), json.loads(sys.argv[3]))
        return
    home = Path(sys.argv[1])
    data = home / ".local/share"
    data.mkdir(parents=True, mode=0o700)
    os.environ["XDG_DATA_HOME"] = str(data)
    # Start a new bus even if the outer test runner belongs to a desktop session.
    for variable in [
        "DBUS_SESSION_BUS_ADDRESS",
        "DBUS_SESSION_BUS_PID",
        "GNOME_KEYRING_CONTROL",
        "GNOME_KEYRING_PID",
        "SSH_AUTH_SOCK",
        "DISPLAY",
        "WAYLAND_DISPLAY",
    ]:
        os.environ.pop(variable, None)
    index_parent = Path(os.environ["XDG_CONFIG_HOME"]) / "osp"
    index_parent_mode = index_parent.stat().st_mode & 0o777
    try:
        result = subprocess.run(
            [
                "dbus-run-session",
                "--",
                sys.executable,
                str(Path(__file__).resolve()),
                "--session",
                *sys.argv[1:],
            ],
            text=True,
            capture_output=True,
            timeout=120,
        )
    finally:
        index_parent.chmod(index_parent_mode)
    if result.returncode:
        raise RuntimeError("isolated keyring session failed:\n" + result.stderr)
    sys.stdout.write(result.stdout)


if __name__ == "__main__":
    main()
