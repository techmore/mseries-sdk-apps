#!/usr/bin/env python3
import json, socket, sys
path = next(x.split('=', 1)[1] for x in sys.argv if x.startswith('--socket='))
with socket.socket(socket.AF_UNIX) as server:
    server.bind(path)
    server.listen(1)
    connection, _ = server.accept()
    with connection, connection.makefile('r') as stream:
        for line in stream:
            request = json.loads(line)
            method = request['method']
            if method == 'hang':
                continue
            if method == 'drop':
                break
            response = {'jsonrpc': '2.0', 'id': request['id']}
            if method == 'fail':
                response['error'] = {'code': -1, 'message': 'fixture failure'}
            else:
                response['result'] = [{'number': '+15550000000'}]
            connection.sendall((json.dumps(response) + '\n').encode())
