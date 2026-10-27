#!/usr/bin/env python3
"""A stand-in for `signal-cli --data-dir D daemon --socket=S`.

Speaks the same line-delimited JSON-RPC as signal-cli for the methods this app
uses and never touches the network. Every `send` is appended to $FAKE_SIGNAL_LOG.
Message text steers the outcome: "FAIL" is an error answer, "SILENT" gets no answer,
"EMPTY" is accepted by no recipient.
"""
import json, os, signal, socketserver, sys, threading, time

path = next(x.split('=', 1)[1] for x in sys.argv if x.startswith('--socket='))
log = os.environ.get('FAKE_SIGNAL_LOG')
lock = threading.Lock()
connections = []
clock = [1790000000000]


def record(entry):
    if log:
        with lock, open(log, 'a') as stream:
            stream.write(json.dumps(entry) + '\n')


class Handler(socketserver.StreamRequestHandler):
    def handle(self):
        connections.append(self.wfile)
        for line in self.rfile:
            request = json.loads(line)
            method, params = request['method'], request.get('params', {})
            response = {'jsonrpc': '2.0', 'id': request['id']}
            if method == 'send':
                record({'method': method, 'params': params})
                text = params.get('message', '')
                if 'SILENT' in text:
                    continue
                if 'FAIL' in text:
                    response['error'] = {'code': -1, 'message': 'fixture failure'}
                else:
                    with lock:
                        clock[0] += 1000
                        stamp = clock[0]
                    ok = 'EMPTY' not in text
                    response['result'] = {'timestamp': stamp, 'results': [{'type': 'SUCCESS'}] if ok else []}
            elif method == 'listAccounts':
                response['result'] = [{'number': '+15550000001'}]
            else:
                response['error'] = {'code': -32601, 'message': 'unknown method'}
            self.wfile.write((json.dumps(response) + '\n').encode())
            self.wfile.flush()


class Server(socketserver.ThreadingMixIn, socketserver.UnixStreamServer):
    daemon_threads = True


def stop(*_):
    try:
        os.unlink(path)
    except OSError:
        pass
    os._exit(0)


signal.signal(signal.SIGTERM, stop)
signal.signal(signal.SIGINT, stop)
record({'method': 'started', 'args': sys.argv[1:]})
Server(path, Handler).serve_forever()
