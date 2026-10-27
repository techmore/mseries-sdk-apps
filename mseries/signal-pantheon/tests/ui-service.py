#!/usr/bin/env python3
import json, socket, sys, time, os, threading, struct
timers = {}
call_id = -9223372036854775807
long_text = os.environ.get("SIGNAL_TEST_LONG") == "1"
alice_name = "Alice Fixture" if not long_text else "Alice — a very long contact name with Unicode Ω and several additional words"
group_name = "Friends Fixture" if not long_text else "Friends — a long group name that should remain within the sidebar"
path = next(x.split('=', 1)[1] for x in sys.argv if x.startswith('--socket='))
video_path = os.path.join(os.environ['XDG_RUNTIME_DIR'], 'fixture-video.sock')
def echo_video():
    with socket.socket(socket.AF_UNIX) as listener:
        listener.bind(video_path); listener.listen(5)
        while True:
            client,_=listener.accept()
            def echo(client):
                with client:
                    while True:
                        data=client.recv(65536)
                        if not data: break
                        try: client.sendall(data)
                        except OSError: break
            threading.Thread(target=echo,args=(client,),daemon=True).start()
threading.Thread(target=echo_video,daemon=True).start()
with socket.socket(socket.AF_UNIX) as server:
    server.bind(path)
    server.listen(1)
    connection, _ = server.accept()
    with connection, connection.makefile('r') as stream:
        def send(value):
            connection.sendall((json.dumps(value) + '\n').encode())
        for line in stream:
            r = json.loads(line)
            method, params = r['method'], r.get('params', {})
            if method == 'send' and params.get('message') == 'fixture-fail':
                send({'jsonrpc': '2.0', 'id': r['id'], 'error': {'code': -1, 'message': 'Fixture send failure'}})
                continue
            if method == 'pantheonCapabilities':
                result = {'expiration':1,'readSync':1,'voiceCalls':1,'videoCalls':1,'groupCalls':1,'viewOnce':1}
            elif method == 'subscribeCallEvents':
                result = 7
            elif method == 'startCall':
                result = {'callId':call_id,'isOutgoing':True,'state':'RINGING_OUTGOING','inputDeviceName':'signal_input_1','outputDeviceName':'signal_output_1','video':params.get('video',False),'videoSocket':video_path,'groupId':params.get('groupId',''),'participants':[{'demuxId':0,'uuid':'fixture-peer','audioMuted':False,'videoMuted':False}]}
                call_id += 1
            elif method in ('setCallVideo','setCallAudio'):
                result = {'success':True}
            elif method in ('hangupCall','rejectCall'):
                result = {}
            elif method == 'acceptCall':
                result = {'callId':params['callId'],'state':'CONNECTING','inputDeviceName':'signal_input_1','outputDeviceName':'signal_output_1'}
            elif method in ('updateContact','updateGroup'):
                timers[params.get('recipient',params.get('groupId'))] = params['expiration']
                result = {}
            elif method == 'listAccounts':
                result = [{'number': '+15550000000', 'aci': 'self-uuid'}]
            elif method == 'listContacts':
                result = [{'name': alice_name, 'number': '+15550000001', 'uuid': 'alice-uuid', 'messageExpirationTime': 0}]
            elif method == 'listGroups':
                result = [{'name': group_name, 'id': 'fixture-group', 'isMember': True, 'members': [{'number': '+15550000001'}]}]
            elif method in ('send', 'sendReaction', 'sendReceipt', 'remoteDelete'):
                destination = params.get('groupId',params.get('recipient',[''])[0] if isinstance(params.get('recipient'),list) else params.get('recipient',''))
                result = {'timestamp': int(time.time() * 1000), 'results': [{'type': 'SUCCESS'}], 'expirationByRecipient':{destination:timers.get(destination,0)},'expirationStartTimestamp':int(time.time()*1000)}
            else:
                send({'jsonrpc': '2.0', 'id': r['id'], 'error': {'code': -1, 'message': 'Unsupported fixture method'}})
                continue
            if method == 'listAccounts':
                send({'jsonrpc': '2.0', 'method': 'receive', 'params': {'account': '+15550000000', 'envelope': {
                    'sourceNumber': '+15550000001', 'sourceUuid': 'alice-uuid', 'sourceName': alice_name,
                    'dataMessage': {'timestamp': int(time.time() * 1000) - 1000, 'message': 'Incoming fixture message' if not long_text else 'A long incoming message with Unicode Ω. ' * 8 + 'longword' * 20}}}})
            # Exercise an event that arrives before the start RPC's continuation.
            if method == 'startCall' and params.get('video'):
                send({'jsonrpc':'2.0','method':'callEvent','params':{'subscription':7,'result':{**result,'isOutgoing':False,'state':'CONNECTED'}}})
            send({'jsonrpc': '2.0', 'id': r['id'], 'result': result})
            if method in ('startCall','acceptCall'):
                send({'jsonrpc':'2.0','method':'callEvent','params':{'subscription':7,'result':{
                    **result,'state':'CONNECTED'}}})
