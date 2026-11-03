"""Dependency-free web interface for the current Rust collection engine (Python 3.11+)."""
from __future__ import annotations
import argparse
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import subprocess
import threading
import tomllib
from urllib.parse import parse_qs, urlsplit
import webbrowser

ROOT = Path(__file__).resolve().parents[1]


def engine_path():
    candidates = [os.environ.get('TPSEARCH_EXE'), str(ROOT / 'target' / 'release' / 'tpsearch.exe'),
                  str(ROOT / 'target' / 'release' / 'tpsearch'), r'D:\GitHub\Rust-Search-\target\release\tpsearch.exe']
    for path in candidates:
        try:
            if path and Path(path).is_file():
                return path
        except OSError:
            continue
    raise ValueError('Build with cargo build --release, or set TPSEARCH_EXE to your existing binary.')


def config_path(explicit=None):
    if explicit:
        return Path(explicit).expanduser().resolve()
    if os.environ.get('TPSEARCH_CONFIG'):
        return Path(os.environ['TPSEARCH_CONFIG']).expanduser().resolve()
    # A copied repo next to the central hub shares its configured corpus.
    hub = ROOT.parent / 'tpsearch.toml'
    if hub.exists():
        return hub
    if os.name == 'nt':
        return Path(os.environ['APPDATA']) / 'tpsearch' / 'config.toml'
    return Path(os.environ.get('XDG_CONFIG_HOME', Path.home() / '.config')) / 'tpsearch' / 'config.toml'


def collections(config):
    if not config.is_file():
        return []
    with config.open('rb') as stream:
        return sorted(tomllib.load(stream).get('collections', {}))


def search(engine, config, values):
    query = values.get('q', '').strip()
    if not query or len(query) > 2000:
        raise ValueError('Enter a search of 1 to 2000 characters.')
    selected = values.get('collection', '')
    if selected not in collections(config):
        raise ValueError('Choose a configured collection. Create/index it with tpsearch first.')
    limit = max(1, min(200, int(values.get('limit', '30'))))
    command = [engine, 'search', '--config', str(config), '-c', selected, '--json', '--limit', str(limit)]
    for key, flag in [('type', '--type'), ('path', '--path'), ('representation', '--representation'),
                      ('after', '--after'), ('before', '--before')]:
        if values.get(key):
            command += [flag, values[key]]
    if values.get('meta'):
        if '=' not in values['meta']:
            raise ValueError('Metadata filter must be key=value, for example domain=Theology.')
        command += ['--meta', values['meta']]
    for key in ['phrase', 'any']:
        if values.get(key) == 'true':
            command += ['--' + key]
    command += ['--', query]
    result = subprocess.run(command, capture_output=True, text=True, encoding='utf-8',
                            errors='replace', timeout=90, creationflags=getattr(subprocess, 'CREATE_NO_WINDOW', 0))
    if result.returncode:
        raise ValueError(result.stderr.strip() or 'Rust search failed. Check the collection index.')
    return json.loads(result.stdout)


class Handler(BaseHTTPRequestHandler):
    def respond(self, code, content, mime='application/json; charset=utf-8'):
        body = content.encode('utf-8') if isinstance(content, str) else content
        self.send_response(code)
        self.send_header('Content-Type', mime)
        self.send_header('Content-Length', str(len(body)))
        self.send_header('Cache-Control', 'no-store')
        self.send_header('X-Content-Type-Options', 'nosniff')
        self.send_header('Content-Security-Policy', "default-src 'self'; script-src 'self' 'unsafe-inline'; style-src 'self' 'unsafe-inline'; connect-src 'self'; frame-ancestors 'none'")
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self):
        expected = f'127.0.0.1:{self.server.server_port}'
        if self.headers.get('Host') != expected:
            self.respond(403, json.dumps({'error': 'Use the printed localhost URL.'}))
            return
        request = urlsplit(self.path)
        try:
            if request.path == '/':
                self.respond(200, (ROOT / 'ui' / 'index.html').read_bytes(), 'text/html; charset=utf-8')
            elif request.path == '/api/collections':
                self.respond(200, json.dumps({'collections': collections(self.server.config), 'config': str(self.server.config)}))
            elif request.path == '/api/search':
                values = {k: v[-1] for k, v in parse_qs(request.query).items()}
                self.respond(200, json.dumps(search(self.server.engine, self.server.config, values)))
            else:
                self.respond(404, json.dumps({'error': 'Not found'}))
        except (ValueError, OSError, subprocess.TimeoutExpired) as exc:
            self.respond(400, json.dumps({'error': str(exc)}))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--config')
    parser.add_argument('--port', type=int, default=9800)
    parser.add_argument('--no-browser', action='store_true')
    args = parser.parse_args()
    server = ThreadingHTTPServer(('127.0.0.1', args.port), Handler)
    server.engine = engine_path()
    server.config = config_path(args.config)
    url = f'http://127.0.0.1:{server.server_port}'
    print(f'tpsearch GUI: {url}\nConfig: {server.config}\nEngine: {server.engine}\nPress Ctrl+C to stop.', flush=True)
    if not args.no_browser:
        threading.Timer(0.5, lambda: webbrowser.open(url)).start()
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        pass
    finally:
        server.server_close()


if __name__ == '__main__':
    main()
