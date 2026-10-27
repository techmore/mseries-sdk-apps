#!/usr/bin/env python3
"""Two real RingRTC tunnels; synthetic audio only; private audio server and bus."""
import argparse, base64, json, os, pathlib, queue, struct, subprocess, tempfile, threading, time, socket
parser = argparse.ArgumentParser()
parser.add_argument('--tunnel', required=True)
parser.add_argument('--pulse-tools', required=True)
parser.add_argument('--report', required=True)
parser.add_argument('--bridge')
parser.add_argument('--video', action='store_true')
parser.add_argument('--video-bridge')
parser.add_argument('--group-probe', action='store_true')
parser.add_argument('--group-only', action='store_true')
args = parser.parse_args()
processes = []
report = {'scope':'private PipeWire/Pulse, two local tunnel peers, synthetic audio; no Signal account', 'states':[]}
with tempfile.TemporaryDirectory(prefix='signal-call-media-') as folder:
    root = pathlib.Path(folder)
    env = os.environ.copy()
    env.update(XDG_RUNTIME_DIR=folder, PIPEWIRE_RUNTIME_DIR=folder, PIPEWIRE_REMOTE='pipewire-0',
               XDG_CONFIG_HOME=folder+'/config', XDG_STATE_HOME=folder+'/state',
               PULSE_SERVER='unix:'+folder+'/pulse/native', DBUS_SYSTEM_BUS_ADDRESS='unix:path='+folder+'/no-system-bus',
               SIGNAL_PRIVATE_AUDIO_TEST='1', PATH=str(pathlib.Path(args.pulse_tools).resolve())+':'+env['PATH'])
    logs = open(pathlib.Path(args.report).with_suffix('.log'), 'w')
    def spawn(command, **kw):
        p = subprocess.Popen(command, env=env, stderr=logs, **kw); processes.append(p); return p
    def run(command): return subprocess.check_output(command, env=env, stderr=logs, text=True)
    def wait_file(path):
        for _ in range(100):
            if path.exists(): return
            time.sleep(.05)
        raise TimeoutError(str(path))
    try:
        bus = subprocess.Popen(['dbus-daemon','--nofork','--print-address=1','--config-file='+str(pathlib.Path(__file__).with_name('private-bus.conf'))], stdout=subprocess.PIPE, stderr=logs, text=True)
        processes.append(bus); env['DBUS_SESSION_BUS_ADDRESS'] = bus.stdout.readline().strip()
        spawn(['pipewire'], stdout=logs); wait_file(root/'pipewire-0')
        spawn(['pipewire-pulse'], stdout=logs); wait_file(root/'pulse/native')
        spawn(['wireplumber','--profile','policy'], stdout=logs); time.sleep(1)
        run(['pactl','load-module','module-null-sink','sink_name=fixture_output'])
        run(['pactl','load-module','module-null-sink','sink_name=fixture_microphone'])
        run(['pactl','load-module','module-remap-source','source_name=fixture_input','master=fixture_microphone.monitor'])
        run(['pactl','set-default-sink','fixture_output']); run(['pactl','set-default-source','fixture_input'])
        events = queue.Queue(); peers = {}; ready = set(); connected = set()
        call_id = 9223372036854775811
        identities = {'alice':base64.b64encode(os.urandom(32)).decode(), 'bob':base64.b64encode(os.urandom(32)).decode()}
        devices = {'alice':2, 'bob':3}
        ids = {'alice':'11111111-1111-4111-8111-111111111111','bob':'22222222-2222-4222-8222-222222222222'}
        def read_events(name, process):
            for line in process.stdout:
                try: events.put((name,json.loads(line)))
                except ValueError: pass
        def send(name, event):
            peers[name].stdin.write(json.dumps(event)+'\n'); peers[name].stdin.flush()
        for name in ('alice','bob'):
            p = spawn([str(pathlib.Path(args.tunnel).resolve())], stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True)
            peers[name] = p
            threading.Thread(target=read_events,args=(name,p),daemon=True).start()
            send(name, {'call_id':call_id,'is_outgoing':name=='alice','local_device_id':devices[name],
                        'input_device_name':'signal_input_'+name,'output_device_name':'signal_output_'+name, **({'video_socket':str(root/(name+'.sock'))} if args.video else {})})
        until = time.monotonic()+15
        while len(ready)<2:
            name,event=events.get(timeout=max(.1,until-time.monotonic()))
            if event['type']=='ready': ready.add(name)
            if time.monotonic()>until: raise TimeoutError('audio readiness')
        send('alice',{'type':'createOutgoingCall','callId':call_id,'peerId':ids['bob'],'video':args.video})
        send('alice',{'type':'proceed','callId':call_id,'hideIp':False,'iceServers':[]})
        until=time.monotonic()+30
        while len(connected)<2 and time.monotonic()<until:
            name,event=events.get(timeout=max(.1,until-time.monotonic()))
            other='bob' if name=='alice' else 'alice'; kind=event['type']
            if kind=='sendOffer':
                send(other,{'type':'receivedOffer','callId':call_id,'peerId':ids[name], 'opaque':event['opaque'],
                            'senderDeviceId':devices[name],'age':0,'video':args.video,'senderIdentityKey':identities[name],'receiverIdentityKey':identities[other]})
                send(other,{'type':'proceed','callId':call_id,'hideIp':False,'iceServers':[]})
            elif kind=='sendAnswer':
                send(other,{'type':'receivedAnswer','opaque':event['opaque'],'senderDeviceId':devices[name],
                            'senderIdentityKey':identities[name],'receiverIdentityKey':identities[other]})
            elif kind=='sendIce':
                send(other,{'type':'receivedIce','senderDeviceId':devices[name], 'candidates':[c['opaque'] for c in event['candidates']]})
            elif kind=='stateChange':
                report['states'].append({'peer':name,'state':event['state']})
                if name=='bob' and event['state']=='Ringing': send(name,{'type':'accept'})
                if event['state']=='Connected': connected.add(name)
                if event['state']=='Ended': raise RuntimeError('peer ended before connected: '+str(event))
            elif kind=='error': raise RuntimeError(event.get('message','tunnel error'))
        assert len(connected)==2, report
        if args.video:
            videos={}
            for name in peers:
                if name!='alice' or not args.video_bridge:
                    client=socket.socket(socket.AF_UNIX,socket.SOCK_STREAM);client.settimeout(12);client.connect(str(root/(name+'.sock')));videos[name]=client
                send(name,{'type':'videoEnabled','enabled':True})
            sending=threading.Event();sending.set()
            def video_source():
                header=struct.pack('!IIII',0x53565031,320,240,0)
                frame=bytes([220,30,70,255])*(320*240)
                while sending.is_set():
                    try:videos['alice'].sendall(header+frame)
                    except OSError:break
                    time.sleep(1/15)
            native_video=None
            if args.video_bridge:
                native_video=spawn([str(pathlib.Path(args.video_bridge).resolve()),str(root/'alice.sock')],stdin=subprocess.PIPE,stdout=subprocess.PIPE,text=True)
                assert native_video.stdout.readline().strip()=='ready'
            else:threading.Thread(target=video_source,daemon=True).start()
            def exact(sock,size):
                data=b''
                while len(data)<size:
                    more=sock.recv(size-len(data))
                    if not more:raise EOFError('video socket closed')
                    data+=more
                return data
            received=[]
            for i in range(5):
                magic,w,h,track=struct.unpack('!IIII',exact(videos['bob'],16));assert magic==0x53565031 and 0<w<=1280 and 0<h<=720
                pixels=exact(videos['bob'],w*h*4);center=pixels[(w*h//2)*4:(w*h//2)*4+4]
                if not args.video_bridge: assert center[0]>150 and center[1]<100 and center[2]<120, list(center)
                else: assert len(set(pixels[::4]))>4, 'Expected synthetic video test pattern'
                received.append({'width':w,'height':h,'track':track,'center_rgba':list(center)})
            sending.clear()
            if native_video:
                native_video.stdin.write('camera-off\n');native_video.stdin.flush();assert native_video.stdout.readline().strip()=='ok'
                native_video.stdin.write('stop\n');native_video.stdin.flush();native_video.wait(timeout=4);assert native_video.returncode==0
            for client in videos.values():client.close()
            report['video']={'result':'PASS','received_frames':received,'synthetic_source':True,'hardware_camera_used':False,'production_video_bridge':bool(args.video_bridge)}
        bridge=None;measurements={}
        if not args.group_only:
            bridge = None
            if args.bridge:
                bridge = spawn([str(pathlib.Path(args.bridge).resolve()),'signal_input_alice','signal_output_bob'],stdin=subprocess.PIPE,stdout=subprocess.PIPE,text=True)
                assert bridge.stdout.readline().strip()=='ready'
            tone_sink = 'fixture_microphone' if bridge else 'sink_for_signal_input_alice'
            recording_source = 'fixture_output.monitor' if bridge else 'signal_output_bob.monitor'
            tone=spawn(['gst-launch-1.0','-q','audiotestsrc','is-live=true','freq=440','volume=0.4','!','audio/x-raw,rate=48000,channels=1','!','audioconvert','!','pulsesink','device='+tone_sink],stdout=logs)
            def capture_segment(label):
                recording=root/(label+'.raw')
                recorder=spawn(['parecord','--device='+recording_source,'--rate=48000','--channels=1','--format=s16le','--latency-msec=20','--raw',str(recording)],stdout=logs)
                time.sleep(3)
                recorder.terminate();recorder.wait(timeout=3)
                data=recording.read_bytes();samples=struct.unpack('<'+'h'*(len(data)//2),data[:len(data)//2*2])
                peak=max(map(abs,samples),default=0)
                rms=(sum(x*x for x in samples)/max(1,len(samples)))**.5
                return {'samples':len(samples),'peak':peak,'rms':rms}
            time.sleep(1)
            measurements={'unmuted':capture_segment('unmuted')}
            assert measurements['unmuted']['samples']>48000 and measurements['unmuted']['peak']>100,measurements
            if bridge:
                def control(command):
                    bridge.stdin.write(command+'\n');bridge.stdin.flush();assert bridge.stdout.readline().strip()=='ok';time.sleep(1)
                control('mute-mic');measurements['mic_muted']=capture_segment('mic-muted')
                assert measurements['mic_muted']['rms']<measurements['unmuted']['rms']*.1,measurements
                control('unmute-mic');control('mute-speaker');measurements['speaker_muted']=capture_segment('speaker-muted')
                assert measurements['speaker_muted']['rms']<measurements['unmuted']['rms']*.1,measurements
                control('unmute-speaker');measurements['restored']=capture_segment('restored')
                assert measurements['restored']['peak']>100,measurements
                bridge.stdin.write('stop\n');bridge.stdin.flush();bridge.wait(timeout=3);assert bridge.returncode==0
            tone.terminate();tone.wait(timeout=3)
        if args.group_probe:
            group_command=[str(pathlib.Path(args.tunnel).resolve())]
            group=spawn(group_command,stdin=subprocess.PIPE,stdout=subprocess.PIPE,text=True)
            peers['group']=group;threading.Thread(target=read_events,args=('group',group),daemon=True).start()
            send('group',{'call_id':44,'is_outgoing':True,'local_device_id':4,'input_device_name':'signal_input_group','output_device_name':'signal_output_group'})
            deadline=time.monotonic()+12
            while True:
                name,event=events.get(timeout=max(.1,deadline-time.monotonic()))
                if name=='group' and event['type']=='ready':break
                assert time.monotonic()<deadline
            user=base64.b64encode(bytes.fromhex('11'*16)).decode()
            member=base64.b64encode(bytes.fromhex('22'*65)).decode()
            proof=base64.b64encode(b'11111111-1111-4111-8111-111111111111:fixture-only').decode()
            send('group',{'type':'groupStart','groupId':base64.b64encode(b'g'*32).decode(),'selfUuid':user,'sfuUrl':'https://fixture.invalid/','proof':proof,'members':[{'userId':user,'memberId':member}]})
            requests=[];joined=False;group_events=[];deadline=time.monotonic()+8
            while time.monotonic()<deadline:
                if group.poll() is not None:
                    report['group_exit_code']=group.returncode
                    break
                try:name,event=events.get(timeout=.5)
                except queue.Empty:continue
                if name!='group':continue
                kind=event['type']
                group_events.append({'type':kind,'state':event.get('state'),'joined':event.get('joined')})
                report['group_debug_events']=group_events
                if kind=='requestProof':send('group',{'type':'groupProof','proof':proof})
                elif kind=='requestMembers':send('group',{'type':'groupMembers','members':[{'userId':user,'memberId':member}]})
                elif kind=='httpRequest':
                    assert event['url'].startswith('https://fixture.invalid/') and event['method'] in ('GET','PUT','POST','DELETE')
                    assert any(k.lower()=='authorization' for k in event['headers']), 'Missing SFU proof header'
                    requests.append({'method':event['method'],'path':event['url'].removeprefix('https://fixture.invalid')})
                    send('group',{'type':'httpResponse','requestId':event['requestId'],'status':401,'body':''})
                elif kind=='groupJoin':joined|=event.get('joined',False)
                if requests and kind=='groupEnded':break
            assert requests and not joined,'Rejected SFU proof must not create a joined group'
            send('group',{'type':'hangup'});group.wait(timeout=8);assert group.returncode==0
            del peers['group']
            report['group_transport']={'result':'PASS','requests':requests,'rejected_proof_never_joined':True,'live_sfu_contacted':False,'scope':'real RingRTC group client with mock 401 responses; no successful SFU join claimed'}
        for name in peers: send(name,{'type':'hangup'})
        for p in peers.values(): p.wait(timeout=8); assert p.returncode==0,p.returncode
        modules=run(['pactl','--format=json','list','modules'])
        assert 'signal_input_alice' not in modules and 'signal_output_bob' not in modules
        report.update(result='PASS',measurements=measurements,production_audio_bridge=bool(bridge),linked_device_ids=[2,3],call_id=str(call_id),owned_virtual_devices_removed=True)
    except Exception as error:
        report.update(result='FAIL',error=str(error)); raise
    finally:
        for p in reversed(processes):
            if p.poll() is None: p.terminate()
            try:p.wait(timeout=3)
            except subprocess.TimeoutExpired:p.kill();p.wait()
        logs.close()
        pathlib.Path(args.report).write_text(json.dumps(report,indent=2)+'\n')
print('PASS: two linked-device RingRTC peers connected, synthetic PCM received and owned audio resources removed')
