"""Loopback HTTP forwarder for real S3 transfer faults; never logs headers or bodies."""
import http.client
import http.server
import socket
import threading
import urllib.parse


class TransferProxy:
    def __init__(self, endpoint):
        target = urllib.parse.urlsplit(endpoint)
        assert target.scheme == 'http' and target.hostname == '127.0.0.1'
        self.target = ('127.0.0.1', target.port)
        self.fault = None
        self.cuts = {'upload': 0, 'download': 0}
        self.forwarded = {'upload': 0, 'download': 0}
        self.completed_parts = []
        self.errors = []
        proxy = self

        class Handler(http.server.BaseHTTPRequestHandler):
            protocol_version = 'HTTP/1.1'

            def log_message(self, *_):
                pass  # Authorization and object paths must never enter logs.

            def transfer(self):
                upstream = None
                self.close_connection = True
                try:
                    upstream = socket.create_connection(proxy.target, timeout=20)
                    upstream.settimeout(30)
                    # Preserve signed Host/Authorization verbatim; one request per connection.
                    headers = ''.join(f'{k}: {v}\r\n' for k, v in self.headers.items() if k.lower() != 'connection')
                    upstream.sendall(f'{self.command} {self.path} HTTP/1.1\r\n{headers}Connection: close\r\n\r\n'.encode('latin1'))
                    query = urllib.parse.parse_qs(urllib.parse.urlsplit(self.path).query)
                    part = query.get('partNumber', [None])[0]
                    cut_upload = proxy.fault == 'upload' and self.command == 'PUT' and part == '2'
                    assert not self.headers.get('Transfer-Encoding'), 'Fixture expects fixed-length SDK request bodies'
                    left = int(self.headers.get('Content-Length', '0'))
                    sent = 0
                    while left:
                        chunk = self.rfile.read(min(left, 65536))
                        if not chunk: raise ConnectionError('Client body truncated')
                        upstream.sendall(chunk)
                        left -= len(chunk); sent += len(chunk)
                        if cut_upload and sent >= 131072:
                            proxy.cuts['upload'] += 1
                            proxy.forwarded['upload'] += sent
                            return  # Close both sockets while the actual second part is incomplete.
                    response = http.client.HTTPResponse(upstream)
                    response.begin()
                    self.send_response_only(response.status, response.reason)
                    for key, value in response.getheaders():
                        if key.lower() not in ('connection', 'transfer-encoding'):
                            self.send_header(key, value)
                    self.send_header('Connection', 'close'); self.end_headers()
                    cut_download = proxy.fault == 'download' and self.command == 'GET' and urllib.parse.urlsplit(self.path).path.endswith('.dump') and response.status == 200
                    received = 0
                    while True:
                        chunk = response.read(65536)
                        if not chunk: break
                        self.wfile.write(chunk); self.wfile.flush(); received += len(chunk)
                        if cut_download and received >= 131072:
                            proxy.cuts['download'] += 1
                            proxy.forwarded['download'] += received
                            return
                    if self.command == 'PUT' and part and response.status == 200:
                        proxy.completed_parts.append(int(part))
                except (OSError, http.client.HTTPException, AssertionError) as error:
                    proxy.errors.append(type(error).__name__)
                finally:
                    if upstream is not None: upstream.close()
                    try: self.connection.shutdown(socket.SHUT_RDWR)
                    except OSError: pass

            do_GET = do_PUT = do_POST = do_DELETE = do_HEAD = transfer

        self.server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Handler)
        self.server.daemon_threads = True
        self.endpoint = f'http://127.0.0.1:{self.server.server_port}'
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)
        self.thread.start()

    def close(self):
        self.server.shutdown(); self.server.server_close(); self.thread.join(timeout=5)
