"""Startup guards must refuse input when an expected screen is absent."""
import json
import tempfile
import threading
import time
import sys
import unittest
from pathlib import Path
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from pathlib import Path
from unittest.mock import Mock
from recover_broker import (
    advance_recovery_startup,
    confirmed_checkpoints,
    wait_for_startup_screen,
)


class RecoveryStartupTest(unittest.TestCase):
    def test_later_restore_is_not_save_evidence(self):
        def row(kind, command, reply=''):
            return {'event_type': kind, 'command': command, 'response': reply,
                    'session_id': 'session', 'branch_id': 'b0',
                    'event_id': kind + command, 'engine_state': 'awaiting_input_or_idle',
                    'metadata': {'name': 'safe'}}
        save = [row('checkpoint', 'save'),
                row('checkpoint_filename', 'safe.sav', 'Ok.')]
        restore = [row('restore', 'restore'),
                   row('checkpoint_filename', 'safe.sav', 'Ok.')]
        self.assertEqual(confirmed_checkpoints(save + restore, 'safe', '/save'), [save[1]])
        self.assertEqual(confirmed_checkpoints(restore, 'safe', '/save'), [])

    def test_quarantined_or_timeout_save_is_refused(self):
        from copy import deepcopy
        pair = [dict(event_id='s:1', event_type='checkpoint', command='save',
                     response='File?', session_id='s', branch_id='b1',
                     engine_state='awaiting_input_or_idle', metadata={'name': 'safe'}),
                dict(event_id='s:2', event_type='checkpoint_filename', command='/save',
                     response='Ok.', session_id='s', branch_id='b1',
                     engine_state='awaiting_input_or_idle', metadata={'name': 'safe'})]
        self.assertEqual(confirmed_checkpoints(pair, 'safe', '/save'), [pair[1]])
        for index in (0, 1):
            bad = deepcopy(pair)
            bad[index]['engine_state'] = 'no_output_timeout'
            self.assertEqual(confirmed_checkpoints(bad, 'safe', '/save'), [])
            self.assertEqual(confirmed_checkpoints(pair, 'safe', '/save',
                                                  [pair[index]['event_id']]), [])
        delayed = dict(pair[1], event_id='s:3', event_type='delayed_output', command=None,
                       metadata={'delayed_for_event_id': 's:2'})
        self.assertEqual(confirmed_checkpoints(pair + [delayed], 'safe', '/save'), [])
        timeout = dict(pair[0], event_id='s:0', event_type='command', command='look',
                       engine_state='no_output_timeout')
        self.assertEqual(confirmed_checkpoints([timeout] + pair, 'safe', '/save'), [])
        timeout['command'] = ''  # Empty Enter is still actual interpreter input.
        self.assertEqual(confirmed_checkpoints([timeout] + pair, 'safe', '/save'), [])

    def fixture(self, response, state='awaiting_input_or_idle'):
        temp = tempfile.TemporaryDirectory(prefix='lanthorn-startup-')
        self.addCleanup(temp.cleanup)
        game = Mock(base=Path(temp.name))
        (game.base / 'latest.json').write_text(json.dumps(
            {'response': response, 'engine_state': state}))
        game.send.return_value = {'response': 'Coral Reef\n>',
                                  'engine_state': 'awaiting_input_or_idle'}
        return game

    def test_identified_keypause_advances(self):
        game = self.fixture('[Press a key.]')
        advance_recovery_startup(game, [self.step()])
        self.assertEqual(game.send.call_args.args[0], '')

    def test_space_keypause_advances(self):
        game = self.fixture('Press SPACE to continue')
        step = {'command': ' ', 'expect_before': 'SPACE', 'expect_after': '>'}
        game.send.return_value = {'response': '>', 'engine_state': 'awaiting_input_or_idle'}
        advance_recovery_startup(game, [step])
        self.assertEqual(game.send.call_args.args[0], ' ')

    def test_delayed_title_screen_is_polled(self):
        game = self.fixture('booting...', state='awaiting_input_or_idle')
        path = game.base / 'latest.json'

        def arrive():
            time.sleep(0.2)
            path.write_text(json.dumps(
                {'response': '[Press a key.]', 'engine_state': 'awaiting_input_or_idle'}))

        threading.Thread(target=arrive, daemon=True).start()
        latest = wait_for_startup_screen(game, '[Press a key.]', timeout=2.0)
        self.assertIn('[Press a key.]', latest['response'])

    def step(self):
        return {'command': '', 'expect_before': '[Press a key.]',
                'expect_after': 'Coral Reef'}

    def test_unexpected_screen_or_timeout_sends_nothing(self):
        for response, state in [('Other menu', 'awaiting_input_or_idle'),
                                ('[Press a key.]', 'no_output_timeout')]:
            game = self.fixture(response, state)
            with self.assertRaises(AssertionError):
                advance_recovery_startup(game, [self.step()], timeout=0.2)
            game.send.assert_not_called()

    def test_parser_command_is_refused(self):
        game = self.fixture('[Press a key.]')
        with self.assertRaises(AssertionError):
            advance_recovery_startup(game, [dict(self.step(), command='restore')])
        game.send.assert_not_called()

    def test_unexpected_result_stops_following_steps(self):
        game = self.fixture('[Press a key.]')
        game.send.return_value = {'response': '', 'engine_state': 'no_output_timeout'}
        with self.assertRaises(AssertionError):
            advance_recovery_startup(game, [self.step(), self.step()])
        self.assertEqual(game.send.call_count, 1)


if __name__ == '__main__':
    unittest.main()
