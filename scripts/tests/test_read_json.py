import json
import tempfile
import threading
import time
import unittest
from pathlib import Path
import sys

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from play_broker import read_json, write_json


class ReadJsonTest(unittest.TestCase):
    def test_reads_after_delayed_complete_write(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / 'latest.json'

            def writer():
                time.sleep(0.15)
                path.write_text('')  # empty race
                time.sleep(0.05)
                write_json(path, {'ok': True})

            threading.Thread(target=writer).start()
            self.assertEqual(read_json(path, timeout=2.0), {'ok': True})

    def test_raises_when_empty_past_timeout(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / 'latest.json'
            path.write_text('')
            with self.assertRaises(json.JSONDecodeError):
                read_json(path, timeout=0.2)


if __name__ == '__main__':
    unittest.main()
