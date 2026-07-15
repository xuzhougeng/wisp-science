import json
import subprocess
import sys
import unittest
from pathlib import Path


class KernelWorkerTests(unittest.TestCase):
    def test_linecache_keeps_only_recent_cells(self):
        worker = subprocess.Popen(
            [sys.executable, str(Path(__file__).with_name("kernel_worker.py"))],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
            text=True,
        )
        try:
            self.assertEqual(json.loads(worker.stdout.readline())["type"], "ready")

            for index in range(70):
                request = {
                    "type": "execute",
                    "id": str(index),
                    "code": (
                        "len([key for key in __import__('linecache').cache "
                        "if str(key).startswith('<wisp-kernel:')])"
                    ),
                }
                worker.stdin.write(json.dumps(request) + "\n")
                worker.stdin.flush()
                while True:
                    response = json.loads(worker.stdout.readline())
                    if response.get("type") == "result" and response.get("id") == str(index):
                        break

            self.assertEqual(response["stdout"].strip(), "64")
            worker.stdin.close()
            self.assertEqual(worker.wait(timeout=5), 0)
        finally:
            if worker.poll() is None:
                worker.kill()
                worker.wait()
            if not worker.stdin.closed:
                worker.stdin.close()
            worker.stdout.close()


if __name__ == "__main__":
    unittest.main()
