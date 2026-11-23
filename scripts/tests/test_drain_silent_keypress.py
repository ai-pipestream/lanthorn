"""Silent keypresses must still count as awaiting input."""
import sys
import unittest
from pathlib import Path
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from unittest.mock import MagicMock, patch
import play_broker


class DrainSilentKeypressTest(unittest.TestCase):
    def test_empty_stdout_while_waiting_for_stdin_is_idle(self):
        game = play_broker.Game.__new__(play_broker.Game)
        game.config = {'response_timeout_seconds': 1}
        game.proc = MagicMock()
        game.proc.poll.return_value = None
        game.selector = MagicMock()
        game.selector.select.return_value = []
        with patch.object(game, 'waiting_for_stdin', return_value=True):
            data, state = game.drain()
        self.assertEqual(data, b'')
        self.assertEqual(state, 'awaiting_input_or_idle')

    def test_empty_stdout_without_stdin_wait_times_out(self):
        game = play_broker.Game.__new__(play_broker.Game)
        game.config = {'response_timeout_seconds': 0.2}
        game.proc = MagicMock()
        game.proc.poll.return_value = None
        game.selector = MagicMock()
        game.selector.select.return_value = []
        with patch.object(game, 'waiting_for_stdin', return_value=False):
            data, state = game.drain()
        self.assertEqual(data, b'')
        self.assertEqual(state, 'no_output_timeout')


if __name__ == '__main__':
    unittest.main()
