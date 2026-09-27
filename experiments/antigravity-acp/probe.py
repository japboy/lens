#!/usr/bin/env python3
"""Bounded official ACP probe. Secrets stay in pipes; reports contain synthetic data only."""
import argparse
import base64
import hashlib
import json
import os
from pathlib import Path
import queue
import re
import signal
import struct
import subprocess
import tempfile
import threading
import time
import uuid
import zlib


def png(color):
    def chunk(kind, data):
        return struct.pack('!I', len(data)) + kind + data + struct.pack('!I', zlib.crc32(kind + data))
    return b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', struct.pack('!2I5B', 8, 8, 8, 2, 0, 0, 0)) + chunk(b'IDAT', zlib.compress((b'\x00' + bytes(color) * 8) * 8)) + chunk(b'IEND', b'')


class Rpc:
    def __init__(self, binary, cwd):
        self.process = subprocess.Popen([str(binary)], cwd=cwd, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True, bufsize=1, start_new_session=True)
        self.queue = queue.Queue()
        self.index = 0
        self.updates = []
        self.requests = []
        self.allowed_publication = None
        self.cancel_on_update_sid = None
        self.cancel_observed_chunk = False
        threading.Thread(target=self.read, daemon=True).start()

    def read(self):
        for line in self.process.stdout:
            try:
                self.queue.put(json.loads(line))
            except ValueError:
                continue
        self.queue.put({'process_eof': True})

    def send(self, method, params, notification=False):
        self.index += 1
        value = {'jsonrpc': '2.0', 'method': method, 'params': params}
        if not notification:
            value['id'] = self.index
        self.write(value)
        return self.index

    def write(self, value):
        self.process.stdin.write(json.dumps(value) + '\n')
        self.process.stdin.flush()

    def wait(self, rid, timeout=90):
        start = time.monotonic()
        deadline = start + timeout
        updates = []
        while time.monotonic() < deadline:
            try:
                value = self.queue.get(timeout=deadline-time.monotonic())
            except queue.Empty:
                return {'timeout': True, 'elapsed_seconds': round(time.monotonic()-start, 3), 'updates': updates}
            if value.get('process_eof'):
                return {'process_eof': True, 'updates': updates}
            if 'method' in value and 'id' in value:
                method = value['method']
                self.requests.append({'method': method})
                if method == 'session/request_permission':
                    options = value.get('params', {}).get('options', [])
                    tool = value.get('params', {}).get('toolCall', {})
                    raw = tool.get('rawInput', {})
                    expected = self.allowed_publication
                    identity = tool.get('_meta', {}).get('mcp') == {'server': 'lens_output', 'tool': 'publish_html'}
                    flat = {k: v for k, v in raw.items() if k != 'arguments'}
                    args_match = raw.get('arguments', expected) == expected
                    allowed = expected is not None and identity and flat == expected and args_match
                    choices = [x for x in options if x.get('kind') == ('allow_once' if allowed else 'reject_once')]
                    self.requests[-1].update({'tool_call': tool, 'decision': 'allow_once' if allowed else 'reject_once'})
                    outcome = {'outcome': 'selected', 'optionId': choices[0]['optionId']} if len(choices) == 1 else {'outcome': 'cancelled'}
                    self.write({'jsonrpc': '2.0', 'id': value['id'], 'result': {'outcome': outcome}})
                else:
                    self.write({'jsonrpc': '2.0', 'id': value['id'], 'error': {'code': -32601, 'message': 'This client does not provide filesystem or terminal capabilities'}})
            elif value.get('method') == 'session/update':
                update = value.get('params', {}).get('update', {})
                updates.append(update)
                self.updates.append(update)
                if self.cancel_on_update_sid and update.get('sessionUpdate') == 'agent_message_chunk' and update.get('content', {}).get('text'):
                    self.send('session/cancel', {'sessionId': self.cancel_on_update_sid}, notification=True)
                    self.cancel_on_update_sid = None
                    self.cancel_observed_chunk = True
            elif value.get('id') == rid:
                return {**{k: v for k, v in value.items() if k != 'id'}, 'elapsed_seconds': round(time.monotonic()-start, 3), 'updates': updates}
        return {'timeout': True, 'updates': updates}

    def call(self, method, params, timeout=90):
        return self.wait(self.send(method, params), timeout)

    def close(self):
        if self.process.poll() is None:
            os.killpg(self.process.pid, signal.SIGTERM)
            try:
                self.process.wait(timeout=3)
            except subprocess.TimeoutExpired:
                os.killpg(self.process.pid, signal.SIGKILL)
                self.process.wait()


class Publisher:
    def __init__(self, binary):
        self.process = subprocess.Popen([str(binary)], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True, bufsize=1)
        self.queue = queue.Queue()
        threading.Thread(target=lambda: [self.queue.put(line) for line in self.process.stdout], daemon=True).start()
        self.start = self.call({'op': 'start'})

    def call(self, message):
        self.process.stdin.write(json.dumps(message) + '\n')
        self.process.stdin.flush()
        value = json.loads(self.queue.get(timeout=15))
        if not value.get('ok'):
            raise RuntimeError('Publisher control failed: ' + str(value))
        return value

    def mcp(self):
        return [{'type': 'http', 'name': 'lens_output', 'url': self.start['endpoint_url'], 'headers': [{'name': 'Authorization', 'value': self.start['authorization_header']}]}]

    def close(self):
        if self.process.poll() is None:
            self.process.terminate()
            try:
                self.process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                self.process.kill()
                self.process.wait()


def completed(result):
    return 'result' in result and 'error' not in result and not result.get('timeout') and not result.get('process_eof')


def text_of(result):
    return ''.join(u.get('content', {}).get('text', '') for u in result.get('updates', []) if u.get('sessionUpdate') == 'agent_message_chunk')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--publisher', type=Path)
    parser.add_argument('--phase', choices=['probe', 'live', 'focused'], default='probe')
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--timeout', type=int, default=120)
    args = parser.parse_args()
    if args.phase != 'probe' and not args.publisher:
        parser.error('--publisher is required for live phase')
    cwd = Path(tempfile.mkdtemp(prefix='lens-antigravity-synthetic-'))
    report = {'schema_version': 1, 'phase': args.phase, 'utc': time.strftime('%Y-%m-%dT%H:%M:%SZ', time.gmtime()), 'binary_sha256': hashlib.sha256(args.binary.read_bytes()).hexdigest(), 'cwd': str(cwd), 'tests': {}}
    rpc = Rpc(args.binary, cwd)
    publisher = None
    secrets = []

    def save():
        body = json.dumps(report, ensure_ascii=True, indent=2)
        for secret in secrets:
            body = body.replace(secret, '[REDACTED]')
        body = body.replace(str(Path.home()), '~')
        body = re.sub(r'https://accounts\.google\.com[^\s"<>]*', '[OAUTH URL REDACTED]', body)
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(body + '\n')

    def record(name, value):
        report['tests'][name] = value
        save()
        print(name + ': ' + ('error' if 'error' in value else 'timeout' if value.get('timeout') else 'response'), flush=True)
        return value

    def turn(name, sid, prompt, cancel=False):
        rid = rpc.send('session/prompt', {'sessionId': sid, 'prompt': prompt})
        if cancel:
            rpc.send('session/cancel', {'sessionId': sid}, notification=True)
        result = rpc.wait(rid, args.timeout)
        result['agent_text'] = text_of(result)
        record(name, result)
        if result.get('timeout') or result.get('process_eof'):
            if result.get('timeout'):
                rpc.send('session/cancel', {'sessionId': sid}, notification=True)
                record(name + '_cancel_drain', rpc.wait(rid, 15))
            raise RuntimeError('Prompt did not terminate normally; subsequent scenarios are not run')
        return result

    try:
        init = record('initialize', rpc.call('initialize', {'protocolVersion': 1, 'clientInfo': {'name': 'lens-antigravity-feasibility', 'version': '0.1.0'}, 'clientCapabilities': {}}, 30))
        admission = record('new_without_mcp', rpc.call('session/new', {'cwd': str(cwd), 'mcpServers': []}, 45))
        if args.phase == 'probe' or 'result' not in admission:
            return
        sid = admission['result']['sessionId']
        publisher = Publisher(args.publisher)
        secrets.extend([publisher.start['authorization_header'], publisher.start['authorization_header'].removeprefix('Bearer ')])
        record('lens_sdk_initialize', publisher.call({'op': 'validate_initialize', 'response': init['result']}))
        mcpnew = record('new_with_product_http_mcp', rpc.call('session/new', {'cwd': str(cwd), 'mcpServers': publisher.mcp()}, 60))
        if 'result' in mcpnew:
            sid = mcpnew['result']['sessionId']
            tid = str(uuid.uuid4())
            publisher.call({'op': 'begin', 'turn_id': tid})
            rpc.allowed_publication = {'html': '<article><h1>\u5b9f\u8a3c\u5b9f\u9a13</h1><p>\u308a\u3093\u3054\u306f\u8d64\u3044\u3002</p></article>', 'turn_id': tid}
            turn('japanese_html_publication', sid, [{'type': 'text', 'text': 'This is a synthetic Lens integration experiment. Do not read or write files, run commands, browse, or use tools other than lens_output.publish_html. Publish exactly <article><h1>\u5b9f\u8a3c\u5b9f\u9a13</h1><p>\u308a\u3093\u3054\u306f\u8d64\u3044\u3002</p></article> using publish_html with turn_id ' + tid + '. Then reply in Japanese: \u516c\u958b\u3057\u307e\u3057\u305f\u3002'}])
            rpc.allowed_publication = None
            publication = publisher.call({'op': 'finish'})
            publication['expected_html_matches'] = (publication.get('publication') or {}).get('html') == '<article><h1>\u5b9f\u8a3c\u5b9f\u9a13</h1><p>\u308a\u3093\u3054\u306f\u8d64\u3044\u3002</p></article>'
            record('product_publication', publication)
        if args.phase == 'focused':
            rpc.cancel_on_update_sid = sid
            active = turn('cancel_after_streaming_chunk', sid, [{'type': 'text', 'text': 'Do not use any tools. Write the integers 1 through 1000 in order, each on its own line. Start immediately.'}])
            active['cancel_sent_after_agent_message_chunk'] = rpc.cancel_observed_chunk
            record('cancel_after_streaming_chunk', active)
            again = turn('after_active_cancel', sid, [{'type': 'text', 'text': 'Do not use any tools. Reply exactly: \u518d\u958b\u6210\u529f'}])
            fresh = Rpc(args.binary, cwd)
            try:
                fresh.call('initialize', {'protocolVersion': 1, 'clientInfo': {'name': 'lens-list-feasibility', 'version': '0.1.0'}, 'clientCapabilities': {}}, 30)
                listed = fresh.call('session/list', {'cwd': str(cwd)}, 30)
                if 'result' in listed:
                    listed['result']['sessions'] = [x for x in listed['result'].get('sessions', []) if x.get('sessionId') == sid]
                record('session_list_fresh_process', listed)
            finally:
                fresh.close()
            report['client_requests'] = rpc.requests
            report['criteria'] = {
                'product_html_exact_match': report['tests'].get('product_publication', {}).get('expected_html_matches', False),
                'active_cancelled_after_message': rpc.cancel_observed_chunk and active.get('result', {}).get('stopReason') == 'cancelled',
                'after_active_cancel_exact_match': completed(again) and again['agent_text'].strip() == '\u518d\u958b\u6210\u529f',
            }
            return
        turn('japanese_text', sid, [{'type': 'text', 'text': 'Do not use any tools. Synthetic test: \u65e5\u672c\u8a9e\u3067\u300c\u6625\u306e\u7a7a\u306f\u9752\u3044\u3002\u300d\u3068\u3060\u3051\u7b54\u3048\u3066\u304f\u3060\u3055\u3044\u3002'}])
        turn('image_red_png', sid, [{'type': 'text', 'text': 'Do not use any tools. This is a synthetic uniform-color image. \u753b\u50cf\u306e\u8272\u3092\u65e5\u672c\u8a9e\u306e\u4e00\u8a9e\u3067\u7b54\u3048\u3066\u304f\u3060\u3055\u3044\u3002'}, {'type': 'image', 'mimeType': 'image/png', 'data': base64.b64encode(png((255, 0, 0))).decode()}])
        turn('image_blue_png', sid, [{'type': 'text', 'text': 'Do not use any tools. This is a synthetic uniform-color image. \u753b\u50cf\u306e\u8272\u3092\u65e5\u672c\u8a9e\u306e\u4e00\u8a9e\u3067\u7b54\u3048\u3066\u304f\u3060\u3055\u3044\u3002'}, {'type': 'image', 'mimeType': 'image/png', 'data': base64.b64encode(png((0, 0, 255))).decode()}])
        turn('cancel_immediate', sid, [{'type': 'text', 'text': 'Do not use any tools. Count from 1 to 100 in Japanese.'}], cancel=True)
        turn('after_cancel', sid, [{'type': 'text', 'text': 'Do not use any tools. Reply exactly: \u518d\u958b\u6210\u529f'}])
        listing = rpc.call('session/list', {'cwd': str(cwd)}, 30)
        if 'result' in listing:
            sessions = listing['result'].get('sessions', [])
            listing['result']['sessions'] = [s for s in sessions if s.get('sessionId') == sid]
            listing['observed_session_count'] = len(sessions)
            listing['requested_session_found'] = any(s.get('sessionId') == sid for s in sessions)
        record('session_list', listing)
        history = Rpc(args.binary, cwd)
        try:
            record('history_initialize', history.call('initialize', {'protocolVersion': 1, 'clientInfo': {'name': 'lens-history-feasibility', 'version': '0.1.0'}, 'clientCapabilities': {}}, 30))
            loaded = history.call('session/load', {'sessionId': sid, 'cwd': str(cwd), 'mcpServers': []}, 60)
            loaded['agent_text'] = text_of(loaded)
            loaded['client_requests'] = history.requests
            record('history_load_independent_read_only', loaded)
        finally:
            history.close()
        record('session_resume', rpc.call('session/resume', {'sessionId': sid, 'cwd': str(cwd), 'mcpServers': publisher.mcp()}, 60))
        report['client_requests'] = rpc.requests
        tests = report['tests']
        report['criteria'] = {
            'product_html_exact_match': tests.get('product_publication', {}).get('expected_html_matches', False),
            'japanese_text_exact_match': completed(tests['japanese_text']) and tests['japanese_text']['agent_text'].strip() == '\u6625\u306e\u7a7a\u306f\u9752\u3044\u3002',
            'red_image_identified': completed(tests['image_red_png']) and tests['image_red_png']['agent_text'].strip() in ['\u8d64', '\u8d64\u8272', '\u8d64\u3002', '\u8d64\u8272\u3002'],
            'blue_image_identified': completed(tests['image_blue_png']) and tests['image_blue_png']['agent_text'].strip() in ['\u9752', '\u9752\u8272', '\u9752\u3002', '\u9752\u8272\u3002'],
            'immediate_cancelled': tests['cancel_immediate'].get('result', {}).get('stopReason') == 'cancelled',
            'after_cancel_exact_match': completed(tests['after_cancel']) and tests['after_cancel']['agent_text'].strip() == '\u518d\u958b\u6210\u529f',
            'session_list_found': completed(tests['session_list']) and tests['session_list'].get('requested_session_found', False),
            'independent_history_contains_response': completed(tests['history_load_independent_read_only']) and '\u518d\u958b\u6210\u529f' in tests['history_load_independent_read_only']['agent_text'],
            'independent_history_no_client_requests': completed(tests['history_load_independent_read_only']) and not tests['history_load_independent_read_only']['client_requests'],
        }
    finally:
        rpc.close()
        if publisher:
            publisher.close()
        save()


if __name__ == '__main__':
    main()
