#!/usr/bin/env python3
"""Own offline interpreters; service Muse's file requests and preserve exact evidence."""
import argparse
import datetime
import hashlib
import json
import os
from pathlib import Path
import re
import selectors
import platform
import subprocess
import time
import uuid

ROOT = Path(__file__).resolve().parent.parent

def now():
    return datetime.datetime.now(datetime.timezone.utc).isoformat()

def write_json(path, value):
    temp = path.with_suffix('.tmp')
    temp.write_text(json.dumps(value, ensure_ascii=False, indent=2)+'\n')
    temp.replace(path)

def read_json(path, timeout=2.0):
    """Load JSON, tolerating a brief empty or partial write."""
    path = Path(path)
    deadline = time.monotonic() + timeout
    last_error = None
    while True:
        try:
            text = path.read_text()
            if text.strip():
                return json.loads(text)
            last_error = json.JSONDecodeError('Expecting value', text, 0)
        except (OSError, json.JSONDecodeError) as error:
            last_error = error
        if time.monotonic() >= deadline:
            if last_error is not None:
                raise last_error
            raise json.JSONDecodeError('Expecting value', '', 0)
        time.sleep(0.05)


class Game:
    def __init__(self, config):
        self.config = config
        self.gid = config['game_id']
        self.base = ROOT/'play'/self.gid
        self.base.mkdir(parents=True, exist_ok=False)
        (self.base/'inbox').mkdir(); (self.base/'responses').mkdir(); (self.base/'saves').mkdir()
        self.session = f'{self.gid}-{uuid.uuid4().hex[:10]}'
        self.branch = 0; self.index = 0; self.rows = {}; self.hint_cursor = 0; self.done = False
        self.story_sha = hashlib.sha256(Path(config['story']).read_bytes()).hexdigest()
        self.journal = (self.base/'events.jsonl').open('a', buffering=1)
        self.raw = (self.base/'stdout.raw').open('ab', buffering=0)
        self.start()
        write_json(self.base/'manifest.json', dict(config, session_id=self.session, story_sha256=self.story_sha, started_at=now(), broker_guards={"fleet_session":1,"restart_timeout":1}, dataset_purpose='Synthetic user-command test data, not a player-skill benchmark'))


    def engine_binary(self):
        """Resolve zvm-cli/gvm-cli from PATH, LANTHORN_ENGINE, or this checkout's target dirs."""
        name = self.config['engine']
        env = os.environ.get('LANTHORN_ENGINE')
        if env:
            return Path(env)
        root = Path(__file__).resolve().parents[1]
        for candidate in (root / 'target.noindex' / 'debug' / name,
                          root / 'target' / 'debug' / name,
                          Path(name)):
            if candidate.is_file() or (candidate == Path(name)):
                return candidate
        return Path(name)

    def start(self):
        self.unresolved_input = None
        binary = self.engine_binary()
        argv = [str(binary), '--plain', '--data-dir', str(self.base/'saves')]
        if self.config['engine']=='zvm-cli': argv += ['--sound','off']
        argv.append(self.config['story'])
        self.stderr = (self.base/'stderr.log').open('ab', buffering=0)
        self.proc = subprocess.Popen(argv, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=self.stderr, cwd=self.base, env=dict(os.environ, TERM='dumb', NO_COLOR='1'))
        self.selector = selectors.DefaultSelector(); self.selector.register(self.proc.stdout, selectors.EVENT_READ)
        data, state = self.drain()
        row = self.log('startup', None, data, state, {'origin':'system'})
        if state == 'no_output_timeout':
            self.unresolved_input = row['event_id']

    def waiting_for_stdin(self):
        # These owned single-threaded CLIs read commands only on fd 0. A quiet
        # stdout pipe alone cannot distinguish an input prompt from slow VM work.
        try:
            fields = Path(f'/proc/{self.proc.pid}/syscall').read_text().split()
            read_number = {'x86_64': '0', 'aarch64': '63'}.get(platform.machine())
            return (read_number is not None and fields[0] == read_number
                    and int(fields[1], 16) == 0)
        except (OSError, ValueError, IndexError):
            return False  # Unproven readiness must never admit another input.

    def drain(self):
        buf = bytearray()
        deadline = time.monotonic() + self.config.get('response_timeout_seconds', 180)
        while time.monotonic() < deadline:
            if self.selector.select(0.3):
                chunk = os.read(self.proc.stdout.fileno(), 65536)
                if not chunk:
                    return bytes(buf), 'engine_exited'
                buf.extend(chunk)
            elif self.waiting_for_stdin():
                # Empty stdout is still idle when the engine is blocked on stdin
                # (silent keypress / no-op SPACE in menus).
                return bytes(buf), 'awaiting_input_or_idle'
            elif self.proc.poll() is not None:
                return bytes(buf), 'engine_exited'
        return bytes(buf), 'no_output_timeout'

    def log(self, kind, command, data=b'', state='unchanged', meta=None):
        self.index+=1; offset=self.raw.tell(); self.raw.write(data)
        row={'event_id':f'{self.session}:e{self.index:05}', 'timestamp':now(), 'game_id':self.gid,
             'session_id':self.session,'branch_id':f'b{self.branch}', 'event_index':self.index,
             'event_type':kind,'command':command,'response':data.decode('utf-8',errors='replace'),
             'raw_start':offset,'raw_end':offset+len(data),'response_sha256':hashlib.sha256(data).hexdigest(),
             'story_sha256':self.story_sha,'engine_state':state, 'metadata':meta or {}, 'review_status':'unreviewed'}
        self.rows[row['event_id']]=row; self.journal.write(json.dumps(row,ensure_ascii=False)+'\n')
        write_json(self.base/'latest.json',row)
        return row

    def send(self, command, meta, kind='command'):
        if not isinstance(command,str) or len(command)>200 or '\n' in command or '\r' in command or '/' in command or '\\' in command or '..' in command:
            raise ValueError('command must be one short line, with no filesystem paths')
        if getattr(self, 'unresolved_input', None):
            raise ValueError('Previous interpreter input is unresolved; use await_output without sending another command')
        if self.proc.poll() is not None: raise ValueError('engine exited; restart explicitly')
        self.proc.stdin.write((command+'\n').encode());self.proc.stdin.flush()
        data,state=self.drain()
        row = self.log(kind, command, data, state, meta)
        if state == 'no_output_timeout':
            self.unresolved_input = row['event_id']
        return row

    def handle(self, req):
        if req.get('_fleet_session_id', self.session) != self.session:
            raise ValueError('Request belongs to another interpreter session; no input sent')
        action=req['action'];meta={k:v for k,v in req.items() if k not in ('action','commands')}
        if action == 'await_output':
            previous = getattr(self, 'unresolved_input', None)
            if not previous:
                raise ValueError('No unresolved interpreter input')
            data, state = self.drain()
            row = self.log('delayed_output', None, data, state,
                           dict(meta, delayed_for_event_id=previous,
                                origin='transport_recovery'))
            if state != 'no_output_timeout':
                self.unresolved_input = None
            return {'event': row, 'note': 'No input sent. Delayed evidence remains quarantined for review.'}
        if action=='command':
            commands=req.get('commands',[])
            if not 1<=len(commands)<=5:raise ValueError('supply 1-5 command objects')
            result=[]
            for item in commands:
                if item.get('origin') not in ('exploration','synthetic_probe','guided','recovery') or not item.get('intent'):
                    raise ValueError('each command needs origin and intent')
                row=self.send(item['command'],dict(meta,**{k:v for k,v in item.items() if k!='command'}))
                result.append(row)
                # Stop a queued route if the engine stops or a common rejection appears.
                if row['engine_state']!='awaiting_input_or_idle' or re.search(r"don't (?:understand|know)|not a verb|can't see|cannot see|not here|which do you mean|\*\*\*.*(?:died|dead)",row['response'],re.I): break
            return {'events':result,'queued_commands_not_sent':len(commands)-len(result)}
        if action=='help':
            if not req.get('reason') or not req.get('topic'):raise ValueError('help needs reason and topic')
            refs=req.get('related_event_ids',[])
            if not refs or any(ref not in self.rows for ref in refs):raise ValueError('help must cite real prior events')
            lines=Path(self.config['walkthrough']).read_text(errors='replace').splitlines()
            words=[w for w in re.findall(r'[a-z]{3,}',req['topic'].lower()) if w not in {'the','and','with','help','need','what','next'}]
            if req.get('mode')=='next': start=self.hint_cursor
            else:
                ranked=[(sum(w in ' '.join(lines[i:i+5]).lower() for w in words),i) for i in range(len(lines))]
                score,at=max(ranked,key=lambda x:(x[0],-x[1]))
                start=max(0,at-1) if score else self.hint_cursor
            end=min(len(lines),start+9);excerpt='\n'.join(lines[start:end])[:1000];self.hint_cursor=end
            row=self.log('help',None,meta=dict(meta,hint_source=self.config['walkthrough_url'],line_start=start+1,line_end=end,hint_text=excerpt,source_kind='walkthrough_not_player_observed'))
            return {'event':row,'hint':excerpt,'note':'Small walkthrough excerpt. A suggested solution is not observed game evidence.'}
        if action=='annotate':
            refs=req.get('event_ids',[])
            if not refs or any(ref not in self.rows for ref in refs):raise ValueError('annotations need real event IDs')
            return {'event':self.log('annotation',None,meta=dict(meta,label_source='muse_unreviewed'))}
        if action in ('checkpoint','restore'):
            name=req.get('name','')
            if not re.fullmatch(r'[a-zA-Z0-9_-]{1,40}',name):raise ValueError('checkpoint name needs simple letters/digits')
            if getattr(self, 'unresolved_input', None):
                raise ValueError('Unresolved input; use await_output before save or restore')
            if action=='restore':self.branch+=1
            first=self.send('save' if action=='checkpoint' else 'restore',meta,action)
            rows=[first]
            if first['engine_state'] == 'awaiting_input_or_idle' and re.search(r'file|save to|restore from',first['response'],re.I):
                rows.append(self.send(name+'.sav',meta,'checkpoint_filename'))
            return {'events':rows,'note':'Inspect responses; save/restore success is not assumed. Restore creates a new provenance branch.'}
        if action=='restart':
            if getattr(self, 'unresolved_input', None):
                raise ValueError('Unresolved input; use await_output before restart')
            self.close_engine();self.branch+=1;self.start()
            return {'event':self.log('restart',None,meta=meta),'startup':read_json(self.base/'latest.json')}
        if action=='finish':
            refs=req.get('evidence_event_ids',[])
            if not refs or any(ref not in self.rows for ref in refs):raise ValueError('ending needs real response evidence')
            quote=req.get('ending_quote','')
            if not quote or not any(quote in self.rows[r]['response'] for r in refs):raise ValueError('ending quote absent from referenced game output')
            row=self.log('ending_claim',None,meta=meta);self.done=True
            write_json(self.base/'ending-claim.json',row)
            return {'event':row,'note':'Claim recorded, awaiting Terra ending verification. No automatic success label.'}
        if action=='status':return {'latest':read_json(self.base/'latest.json'),'ending_claimed':self.done}
        raise ValueError('unknown action')

    def poll(self):
        for path in sorted((self.base/'inbox').glob('*.json')):
            dest=self.base/'responses'/path.name
            if dest.exists():continue
            try:
                req=json.loads(path.read_text())
            except (OSError,json.JSONDecodeError):continue
            if self.done: result={'error':'ending already claimed; awaiting review'}
            else:
                try:result=self.handle(req)
                except Exception as error:result={'error':str(error)}
            write_json(dest,dict(result,request_file=path.name,processed_at=now()))

    def close_engine(self):
        if self.proc.poll() is None:
            self.proc.terminate()
            try:self.proc.wait(timeout=2)
            except subprocess.TimeoutExpired:self.proc.kill();self.proc.wait()
        self.selector.close();self.stderr.close()
        self.proc.stdin.close()
        self.proc.stdout.close()

if __name__=='__main__':
    parser=argparse.ArgumentParser();parser.add_argument('config',type=Path);parser.add_argument('--hours',type=float,default=3)
    args=parser.parse_args();games=[]
    try:
        for item in json.loads(args.config.read_text())['games']:games.append(Game(item))
        write_json(ROOT/'control'/('broker-'+args.config.stem+'-status.json'),{'pid':os.getpid(),'state':'running','started_at':now(),'games':[g.gid for g in games]})
        end=time.monotonic()+args.hours*3600
        while time.monotonic()<end and not (ROOT/'control'/'stop-broker').exists():
            for game in games:
                if game.index<2500:game.poll()
            if all(g.done for g in games):break
            time.sleep(0.15)
    finally:
        for game in games:game.close_engine()
        write_json(ROOT/'control'/('broker-'+args.config.stem+'-status.json'),{'pid':os.getpid(),'state':'stopped','stopped_at':now()})
