#!/usr/bin/env python3
"""Exercise the real signaling-only child without pairing or media resources."""
import argparse,base64,json,os,pathlib,selectors,struct,subprocess,tempfile
p=argparse.ArgumentParser();p.add_argument('--tunnel',required=True);p.add_argument('--report',required=True);a=p.parse_args()
with tempfile.TemporaryDirectory(prefix='signal-ring-probe-') as root:
 env=os.environ.copy();env.update(XDG_RUNTIME_DIR=root,PULSE_SERVER='unix:/nonexistent/signal-ring-no-audio')
 proc=subprocess.Popen([str(pathlib.Path(a.tunnel).resolve())],stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=subprocess.DEVNULL,env=env)
 sel=selectors.DefaultSelector();sel.register(proc.stdout,selectors.EVENT_READ)
 def receive():
  assert sel.select(5),'monitor output timed out'
  return json.loads(proc.stdout.readline())
 def send(v):
  proc.stdin.write((json.dumps(v)+'\n').encode());proc.stdin.flush();return receive()
 try:
  assert send(dict(call_id=0,is_outgoing=False,local_device_id=1,ring_monitor=True))['type']=='ringMonitorReady'
  group=bytes(range(32));rid=-(2**63)+1
  def intention(action):
   payload=b'\x0a\x20'+group+b'\x10'+bytes([action])+b'\x19'+struct.pack('<q',rid)
   return base64.b64encode(b'\x12'+bytes([len(payload)])+payload).decode()
  def decode(opaque,age=0):return send(dict(type='receivedOpaque',token='fixture',age=age,opaque=opaque))
  ring=decode(intention(0));assert ring['action']=='Ring' and ring['ringId']==rid
  assert decode(intention(0),60001)['type']=='ringIgnored'
  assert decode(intention(1))['action']=='Cancelled'
  for opaque in ['bad!',base64.b64encode(bytes(16385)).decode()]:assert decode(opaque)['type']=='ringIgnored'
  for action in ['Accepted','Declined','Busy']:
   reply=send(dict(type='ringResponse',token='response',groupId=base64.b64encode(group).decode(),ringId=rid,response=action))
   decoded=decode(reply['opaque']);assert decoded['kind']=='response' and decoded['action']==action and decoded['ringId']==rid
  descriptors=[]
  for fd in pathlib.Path(f'/proc/{proc.pid}/fd').iterdir():
   try:descriptors.append(os.readlink(fd))
   except FileNotFoundError:pass
  assert not any(x.startswith(('socket:','/dev/snd','/dev/video')) for x in descriptors),descriptors
  pathlib.Path(a.report).write_text(json.dumps(dict(result='PASS',scope='real local tunnel; no account or media',checks=['signed ring ID','stale ring rejection','cancellation','malformed/oversize rejection','Accepted/Declined/Busy protobuf roundtrip','no socket/audio/video descriptors']),indent=2)+'\n')
 finally:
  proc.terminate()
  try:proc.wait(timeout=5)
  except subprocess.TimeoutExpired:proc.kill();proc.wait()
print('PASS: incoming group-ring protocol and signaling-only resources')
