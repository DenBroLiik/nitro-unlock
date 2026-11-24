import importlib.machinery
import importlib.util
import os
from pathlib import Path
import signal
import subprocess
import sys
from tempfile import TemporaryDirectory
import time
from unittest.mock import patch

SCRIPT = Path(__file__).resolve().parents[1] / "nitro-unlock"
loader = importlib.machinery.SourceFileLoader("nitro_unlock", str(SCRIPT))
spec = importlib.util.spec_from_loader(loader.name, loader)
nitro = importlib.util.module_from_spec(spec)
loader.exec_module(nitro)

def fake_paths(base):
    return {
        "MODEL_FILE": base / "model",
        "POWERCAP": base / "powercap",
        "SUPPLY": base / "power_supply",
        "AC": base / "power_supply/ADP1/online",
        "ADAPTER_RATING": base / "adapter",
        "TEMPERATURES": base / "hwmon",
    }

if len(sys.argv) > 1:
    mode, base = sys.argv[1], Path(sys.argv[2])
    for name, value in fake_paths(base).items():
        setattr(nitro, name, value)
    os.environ["PKEXEC_UID"] = str(os.getuid())
    if mode == "wrapper":
        nitro.auth_command = lambda _: [sys.executable, __file__, "worker", str(base)]
        with (base / "wrapper.pid").open("w") as stream:
            print(os.getpid(), file=stream)
        raise SystemExit(nitro.launch([sys.executable, "-c", "import time; time.sleep(3)"]))
    with patch.object(os, "geteuid", return_value=0):
        nitro.worker(restore_stock=mode == "reset")
    raise SystemExit(0)

def setup(base, limit1=50_000_000, limit2=50_000_000):
    zone = base / "powercap/intel-rapl:1"
    files = {
        base / "model": "Nitro NL16-71G",
        base / "adapter": 100000,
        base / "power_supply/ADP1/online": 1,
        base / "hwmon/hwmon0/temp1_label": "Package id 0",
        base / "hwmon/hwmon0/temp1_input": 72000,
        zone / "name": "psys",
        zone / "enabled": 1,
        zone / "constraint_0_name": "long_term",
        zone / "constraint_1_name": "short_term",
        zone / "constraint_0_power_limit_uw": limit1,
        zone / "constraint_1_power_limit_uw": limit2,
    }
    for path, value in files.items():
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(str(value) + "\n")
    return nitro.constraints(zone)

def spawn(base, mode):
    return subprocess.Popen(
        [sys.executable, __file__, mode, str(base)],
        stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True,
    )

with TemporaryDirectory() as temporary:
    base = Path(temporary)
    appimage_env = os.environ | {
        "PYTHONHOME": str(base / "appimage/usr"),
        "PYTHONPATH": str(base / "appimage/usr/share/pyshared"),
    }
    response = subprocess.run(
        [str(SCRIPT), "--help"], env=appimage_env, capture_output=True,
        text=True, check=True, timeout=5,
    )
    assert "nitro-unlock [--] GAME" in response.stdout

    stock = (50_000_000, 50_000_000)
    boosted = (70_000_000, 70_000_000)
    for reason, baseline in (("close", stock), ("close", boosted), ("ac", stock),
                             ("thermal", stock), ("term", stock), ("reset", boosted)):
        paths = setup(base, *baseline)
        process = spawn(base, "reset" if reason == "reset" else "worker")
        assert process.stdout.readline().strip() == nitro.READY
        if reason == "reset":
            assert tuple(map(nitro.number, paths)) == stock
        else:
            assert tuple(map(nitro.number, paths)) == boosted
        if reason == "ac":
            (base / "power_supply/ADP1/online").write_text("0")
        elif reason == "thermal":
            (base / "hwmon/hwmon0/temp1_input").write_text("95000")
        elif reason == "term":
            process.send_signal(signal.SIGTERM)
        else:
            process.stdin.close()
        assert process.wait(timeout=4) == 0, reason
        assert tuple(map(nitro.number, paths)) == stock, reason
        process.stdout.close()
        process.stdin.close()

    paths = setup(base, 47_000_000, 53_000_000)
    with patch.object(nitro, "auth_command", return_value=[sys.executable, __file__, "worker", str(base)]):
        code = "from pathlib import Path; import sys; assert all(Path(x).read_text().strip() == '70000000' for x in sys.argv[1:]); sys.exit(7)"
        assert nitro.launch([sys.executable, "-c", code, *map(str, paths)]) == 7
    assert tuple(map(nitro.number, paths)) == (53_000_000, 47_000_000)

    paths = setup(base)
    with patch.object(nitro, "auth_command", return_value=["/usr/bin/false"]):
        assert nitro.launch(["/usr/bin/true"]) == 0
    assert tuple(map(nitro.number, paths)) == stock

    parent = spawn(base, "wrapper")
    deadline = time.monotonic() + 3
    while tuple(map(nitro.number, paths)) != boosted and time.monotonic() < deadline:
        time.sleep(0.05)
    assert tuple(map(nitro.number, paths)) == boosted
    parent.kill()
    parent.communicate(timeout=5)
    deadline = time.monotonic() + 3
    while tuple(map(nitro.number, paths)) != stock and time.monotonic() < deadline:
        time.sleep(0.05)
    assert tuple(map(nitro.number, paths)) == stock

print("PASS: application exit, initial 70 W, custom limits, AC loss, heat, signals, stock reset, denied authorization and SIGKILL of the wrapper.")
