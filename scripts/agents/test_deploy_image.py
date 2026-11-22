"""Security checks for the agent client's credential transport."""
import contextlib
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
import runpy
import threading
import unittest

client_module = runpy.run_path(str(Path(__file__).with_name('deploy-image.py')))
Client = client_module['Client']
AgentError = client_module['AgentError']
SECRET = 'frx_' + 'a' * 64


@contextlib.contextmanager
def serve(handler):
    server = ThreadingHTTPServer(('127.0.0.1', 0), handler)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        yield 'http://127.0.0.1:' + str(server.server_port)
    finally:
        server.shutdown()
        server.server_close()
        thread.join(timeout=2)


class ClientSecurity(unittest.TestCase):
    def test_rejects_remote_http_url_credentials_and_malformed_keys(self):
        for url in ['http://example.com', 'https://user:password@example.com', 'https://example.com?key=value']:
            with self.subTest(url=url), self.assertRaises(AgentError):
                Client(url, SECRET)
        for key in ['', 'frx_' + 'a' * 63, 'frx_' + 'a' * 63 + '\n']:
            with self.assertRaises(AgentError):
                Client('https://example.com', key)

    def test_redirect_never_receives_key_and_errors_never_echo_it(self):
        visits = []

        class Destination(BaseHTTPRequestHandler):
            def do_GET(self):
                visits.append(self.headers.get('Authorization'))
                self.send_response(200)
                self.end_headers()
                self.wfile.write(b'{}')

            def log_message(self, *args):
                pass

        with serve(Destination) as destination:
            class Redirect(BaseHTTPRequestHandler):
                def do_GET(self):
                    self.send_response(302)
                    self.send_header('Location', destination + '/agent')
                    self.end_headers()
                    self.wfile.write(SECRET.encode())

                def log_message(self, *args):
                    pass

            with serve(Redirect) as origin:
                with self.assertRaises(AgentError) as result:
                    Client(origin, SECRET).call('GET', '/agent')
                self.assertNotIn(SECRET, str(result.exception))
        self.assertEqual(visits, [])


if __name__ == '__main__':
    unittest.main()
