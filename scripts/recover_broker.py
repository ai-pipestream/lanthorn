#!/usr/bin/env python3
"""Explicit checkpoint recovery; append evidence without replaying pending input."""
import argparse
import fcntl
import hashlib
import json
import time
import uuid
from pathlib import Path
from play_broker import Game, ROOT, now, write_json


def confirmed_checkpoints(history, checkpoint, path, quarantined=()):
    """Filename replies are shared by SAVE and RESTORE; require the SAVE request."""
    excluded = set(quarantined)
    pending = set()
    for row in history:
        if row['event_type'] == 'delayed_output':
            excluded.add(row['event_id'])
            excluded.add(row.get('metadata', {}).get('delayed_for_event_id'))
            if row.get('engine_state') != 'no_output_timeout':
                pending.discard(row['session_id'])
        elif row.get('command') is not None:
            if row['session_id'] in pending:
                excluded.add(row['event_id'])
                pending.discard(row['session_id'])
            if row.get('engine_state') == 'no_output_timeout':
                excluded.add(row['event_id'])
                pending.add(row['session_id'])
    return [row for previous, row in zip(history, history[1:])
            if previous['event_type'] == 'checkpoint'
            and previous.get('command') == 'save'
            and previous.get('engine_state') == 'awaiting_input_or_idle'
            and row.get('engine_state') == 'awaiting_input_or_idle'
            and previous['event_id'] not in excluded
            and row['event_id'] not in excluded
            and previous['session_id'] == row['session_id']
            and previous['branch_id'] == row['branch_id']
            and previous['metadata'].get('name') == checkpoint
            and row['event_type'] == 'checkpoint_filename'
            and row['metadata'].get('name') == checkpoint
            and ('Ok.' in row['response'] or
                 f"Saved to '{path}'." in row['response'].splitlines())]


def read_latest(game):
    """Load latest.json, tolerating a brief empty or partial write."""
    path = game.base / 'latest.json'
    deadline = time.monotonic() + 2.0
    while True:
        try:
            text = path.read_text()
            if text.strip():
                return json.loads(text)
        except (OSError, json.JSONDecodeError):
            pass
        if time.monotonic() >= deadline:
            raise AssertionError(f'latest.json unreadable at {path}')
        time.sleep(0.05)


def wait_for_startup_screen(game, expect_before, timeout=8.0):
    """Poll until the expected title/keypause text is present, or time out."""
    deadline = time.monotonic() + timeout
    latest = None
    while time.monotonic() < deadline:
        latest = read_latest(game)
        if (latest.get('engine_state') == 'awaiting_input_or_idle'
                and expect_before in latest.get('response', '')):
            return latest
        time.sleep(0.1)
    state = None if latest is None else latest.get('engine_state')
    excerpt = '' if latest is None else latest.get('response', '')[:240]
    raise AssertionError(
        'Unexpected recovery startup screen: '
        f'expected {expect_before!r} while awaiting_input_or_idle; '
        f'got engine_state={state!r} response={excerpt!r}')


def advance_recovery_startup(game, steps, timeout=8.0):
    """Advance only explicitly identified startup keypauses before restoring.

    Supports Enter (`''`) and SPACE (`' '`) keypauses. Polls latest.json briefly
    so a delayed first paint does not hard-fail the broker.
    """
    for step in steps:
        command = step['command']
        assert command in ('', ' '), 'Recovery startup supports Enter or SPACE keypauses only'
        wait_for_startup_screen(game, step['expect_before'], timeout=timeout)
        row = game.send(command, {'origin': 'recovery',
                                  'intent': 'Advance verified startup keypause before checkpoint restore'},
                        'recovery_startup')
        assert (row['engine_state'] == 'awaiting_input_or_idle'
                and step['expect_after'] in row['response']), (
            'Unexpected recovery startup result: '
            f'expected {step["expect_after"]!r}; '
            f'got engine_state={row["engine_state"]!r} '
            f'response={row.get("response", "")[:240]!r}')


class RecoveredGame(Game):
    def __init__(self, config, checkpoint):
        self.config = config
        self.gid = config['game_id']
        self.base = ROOT / 'play' / self.gid
        self.lock = (self.base / 'recovery.lock').open('a')
        fcntl.flock(self.lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        assert not (self.base / 'ending-claim.json').exists(), 'Already completed'
        history = [json.loads(s) for s in (self.base / 'events.jsonl').read_text().splitlines()]
        previous = json.loads((self.base / 'manifest.json').read_text())
        self.story_sha = hashlib.sha256(Path(config['story']).read_bytes()).hexdigest()
        assert self.story_sha == previous['story_sha256'], 'Story changed'
        files = list((self.base / 'saves').rglob(checkpoint + '.sav.qzl'))
        assert len(files) == 1 and files[0].stat().st_size > 0, 'Missing checkpoint'
        exclusions_path = ROOT / 'reviewed' / 'transport-events.json'
        exclusions = json.loads(exclusions_path.read_text()) if exclusions_path.exists() else {'events': []}
        confirmed = confirmed_checkpoints(history, checkpoint, files[0],
                                          (entry['event_id'] for entry in exclusions['events']))
        assert confirmed, 'Checkpoint has no engine confirmation'
        self.session = f'{self.gid}-{uuid.uuid4().hex[:10]}'
        self.branch = max(int(r['branch_id'][1:]) for r in history) + 1
        self.index = 0
        self.rows = {r['event_id']: r for r in history}
        self.hint_cursor = 0
        self.done = False
        self.journal = (self.base / 'events.jsonl').open('a', buffering=1)
        self.raw = (self.base / 'stdout.raw').open('ab', buffering=0)
        archive = self.base / 'session-history'
        archive.mkdir(exist_ok=True)
        write_json(archive / (previous['session_id'] + '-manifest.json'), previous)
        old_progress = (self.base / 'progress.md').read_text()
        (archive / (self.session + '-previous-progress.md')).write_text(old_progress)
        recovery = dict(previous_session_id=previous['session_id'],
                        checkpoint_event_id=confirmed[-1]['event_id'],
                        checkpoint_path=str(files[0]),
                        checkpoint_sha256=hashlib.sha256(files[0].read_bytes()).hexdigest(),
                        reason=config.get('recovery_reason', 'Previous broker timed out; explicit restore in a new session. Later unsaved actions are historical only.'))
        self.log('session_recovery', None, meta=recovery)
        for request in sorted((self.base / 'inbox').glob('*.json')):
            response = self.base / 'responses' / request.name
            if response.exists():
                continue
            self.log('request_cancelled', None, meta={'request_file': request.name,
                     'reason': 'Old pending request not replayed after recovery',
                     'request_sha256': hashlib.sha256(request.read_bytes()).hexdigest()})
            write_json(response, {'error': 'Cancelled during explicit checkpoint recovery; no command executed. Read recovery.json and current state.',
                                  'request_file': request.name, 'processed_at': now()})
        self.start()
        try:
            advance_recovery_startup(self, config.get('recovery_startup_steps', []))
            result = self.handle({'action': 'restore', 'name': checkpoint,
                                  'reason': 'Explicit service-timeout recovery'})
            reply = result['events'][-1]['response']
            marker = config.get('restore_confirmation_line')
            assert (len(result['events']) == 2
                    and result['events'][-1]['engine_state'] == 'awaiting_input_or_idle'
                    and ('Ok.' in reply or (marker and marker in reply.splitlines()))), 'Restore not confirmed'
            observed = self.send('look', {'origin': 'recovery', 'intent': 'Observe restored location'})
            assert observed['engine_state'] == 'awaiting_input_or_idle', 'Restored interpreter unavailable'
            write_json(self.base / 'recovery.json', dict(recovery, session_id=self.session,
                       branch_id=f'b{self.branch}', restored_at=now(), observed_event_id=observed['event_id']))
            write_json(self.base / 'manifest.json', dict(config, session_id=self.session,
                       story_sha256=self.story_sha, started_at=now(), recovery=recovery,
                       broker_guards={"fleet_session":1,"restart_timeout":1},
                       dataset_purpose='Synthetic user-command test data, not a player-skill benchmark'))
            (self.base / 'progress.md').write_text(
                '# Explicit service-timeout recovery\n\n'
                f'Current session: {self.session}, branch b{self.branch}. Restored {checkpoint} successfully.\n'
                f'Save evidence: {confirmed[-1]["event_id"]}. See recovery.json and latest.json.\n'
                'Old pending requests were cancelled without execution. Never resubmit them blindly.\n'
                'Unsaved actions after the checkpoint did NOT carry over. Check inventory and current room before continuing.\n\n'
                'Observed restored room:\n' + observed['response'] + '\n\n'
                '## Historical notes only; reconcile every state claim with the restored game\n\n' + old_progress)
        except BaseException:
            self.close_engine()
            raise


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('config', type=Path)
    parser.add_argument('--hours', type=float, default=6)
    args = parser.parse_args()
    games = []
    try:
        for item in json.loads(args.config.read_text())['games']:
            item = dict(item)
            checkpoint = item.pop('recovery_checkpoint')
            games.append(RecoveredGame(item, checkpoint))
        deadline = time.monotonic() + args.hours * 3600
        next_save = time.monotonic() + 600
        while time.monotonic() < deadline and not (ROOT / 'control' / 'stop-broker').exists():
            for game in games:
                if game.index < game.config.get('max_events', 2500):
                    game.poll()
            if all(game.done for game in games):
                break
            if time.monotonic() >= next_save:
                for game in games:
                    if (game.config.get('automatic_checkpoints', True)
                            and not game.done and game.proc.poll() is None
                            and not getattr(game, 'unresolved_input', None)):
                        result = game.handle({'action': 'checkpoint',
                            'name': f'auto-{game.session[-10:]}-{game.index}',
                            'reason': 'Periodic recovery checkpoint; inspect response to verify success'})
                        print(json.dumps({'game': game.gid, 'checkpoint_result': result}), flush=True)
                next_save = time.monotonic() + 600
            time.sleep(0.15)
    finally:
        for game in games:
            game.close_engine()
