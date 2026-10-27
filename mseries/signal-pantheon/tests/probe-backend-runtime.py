#!/usr/bin/env python3
"""Exercise the staged JVM daemon against an empty temporary profile; never pair."""
import argparse, json, os, pathlib, socket, subprocess, tempfile, time
p=argparse.ArgumentParser();p.add_argument('--runtime',required=True);p.add_argument('--report',required=True);a=p.parse_args()
r=pathlib.Path(a.runtime).resolve()
with tempfile.TemporaryDirectory(prefix='signal-empty-runtime-') as folder:
    env=os.environ.copy();env.update(JAVA_HOME=str(r/'temurin25'),PATH=str(r/'temurin25/bin')+':'+str(r/'tools/bin')+':'+env['PATH'],SIGNAL_CALL_TUNNEL_BIN=str(r/'tools/bin/signal-call-tunnel'),XDG_RUNTIME_DIR=folder,XDG_DATA_HOME=folder+'/data',XDG_CONFIG_HOME=folder+'/config')
    path=folder+'/service.sock'
    proc=subprocess.Popen([str(r/'signal-cli-arm64/bin/signal-cli'),'--data-dir',folder+'/profile','daemon','--socket='+path],env=env,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
    try:
        for _ in range(200):
            if pathlib.Path(path).exists():break
            if proc.poll() is not None:raise RuntimeError('empty-profile daemon exited')
            time.sleep(.1)
        with socket.socket(socket.AF_UNIX) as sock:
            sock.settimeout(10);sock.connect(path)
            stream=sock.makefile('r')
            results={}
            for number,method in enumerate(['pantheonCapabilities','listAccounts'],1):
                sock.sendall((json.dumps({'jsonrpc':'2.0','method':method,'id':number})+'\n').encode())
                while True:
                    response=json.loads(stream.readline())
                    if response.get('id')==number:break
                assert 'error' not in response,response
                results[method]=response['result']
            assert results['listAccounts']==[],results
            assert results['pantheonCapabilities']=={'expiration':1,'readSync':1,'voiceCalls':1,'videoCalls':1,'groupCalls':1,'viewOnce':1},results
            pathlib.Path(a.report).write_text(json.dumps({'result':'PASS','scope':'real staged daemon, empty temporary profile, no pairing',**results},indent=2)+'\n')
    finally:
        proc.terminate()
        try:proc.wait(timeout=5)
        except subprocess.TimeoutExpired:proc.kill();proc.wait()
print('PASS: staged backend capabilities and empty-profile startup; no account linked')
