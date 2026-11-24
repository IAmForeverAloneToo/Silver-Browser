"""The UA probe: serves a page on 127.0.0.1 that writes the UA and
the client hints to #out (Accept-CH asks for every high-entropy one),
and records each request's headers.
python -I ua_server.py <out-headers.jsonl> <port>"""
import http.server, json, sys

out, port = sys.argv[1], int(sys.argv[2])
PAGE = b'''<!doctype html><meta charset="utf-8"><pre id="out"></pre><script>
(async () => {
  const u = navigator.userAgentData;
  const high = await u.getHighEntropyValues(["architecture", "bitness", "formFactors",
    "fullVersionList", "model", "platformVersion", "uaFullVersion", "wow64"]);
  document.getElementById("out").textContent = JSON.stringify({
    userAgent: navigator.userAgent, appVersion: navigator.appVersion,
    brands: u.brands, mobile: u.mobile, platform: u.platform, high: high,
    toJSON: u.toJSON()});
  fetch("/second");
})();
</script>'''

class Handler(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        with open(out, 'a', encoding='utf-8') as f:
            f.write(json.dumps({'path': self.path, 'headers': dict(self.headers.items())}) + '\n')
        self.send_response(200)
        self.send_header('Content-Type', 'text/html; charset=utf-8')
        self.send_header('Accept-CH', 'Sec-CH-UA-Full-Version-List, Sec-CH-UA-Platform-Version, '
                         'Sec-CH-UA-Arch, Sec-CH-UA-Bitness, Sec-CH-UA-Model, Sec-CH-UA-Full-Version')
        self.end_headers()
        self.wfile.write(PAGE)
    def log_message(self, *a):
        pass

http.server.ThreadingHTTPServer(('127.0.0.1', port), Handler).serve_forever()
