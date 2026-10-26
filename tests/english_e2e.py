"""End-to-end test of English learning output against a loopback mock API."""
import http.server
import json
import os
import sys
sys.stdout.reconfigure(encoding='utf-8')
import pathlib
import re
import subprocess
import tempfile
import threading

ROOT = pathlib.Path(__file__).resolve().parents[1]
BINARY = ROOT / 'target' / 'debug' / ('codegym.exe' if os.name == 'nt' else 'codegym')
class Mock(http.server.BaseHTTPRequestHandler):
    requests = []
    def do_POST(self):
        size = int(self.headers["Content-Length"])
        request = json.loads(self.rfile.read(size))
        text = request["messages"][0]["content"]
        self.__class__.requests.append(text)
        if "Grade the answer" in text:
            result = {"score":5, "feedback":"Correct reasoning.",
                      "what_to_improve":"Explore a boundary case.",
                      "model_answer":"The function prevents negative values."}
        else:
            result = {"questions":[{"topic":"Logic",
                "question":"Why does this function clamp values at zero?",
                "source_path":"logic.ts", "source_line":1,
                "reference_answer":"It prevents negative results.",
                "hint":"Look at Math.max."}]}
        content = json.dumps({"choices":[{"message":{"role":"assistant","content":json.dumps(result)}}]}).encode()
        self.send_response(200)
        self.send_header("Content-Type","application/json")
        self.send_header("Content-Length",str(len(content)))
        self.end_headers()
        self.wfile.write(content)
    def log_message(self,*_):
        pass

server = http.server.HTTPServer(("127.0.0.1",0),Mock)
thread = threading.Thread(target=server.serve_forever,daemon=True)
thread.start()

with tempfile.TemporaryDirectory(prefix="codegym-english-e2e-") as folder:
    path = pathlib.Path(folder)
    repo = path/"repo"
    repo.mkdir()
    def git(*args):
        return subprocess.run(["git","-C",str(repo),*args],check=True,
            stdout=subprocess.PIPE,stderr=subprocess.PIPE).stdout.decode().strip()
    git("init","-q","-b","main")
    (repo/"logic.ts").write_text("export function clamp(n: number){ return n; }\n",encoding="utf-8")
    git("add","logic.ts")
    git("-c","user.name=Test","-c","user.email=test@example.invalid","commit","-qm","initial")
    git("switch","-q","-c","feature")
    (repo/"logic.ts").write_text("export function clamp(n: number){ return Math.max(0,n); }\n",encoding="utf-8")
    git("add","logic.ts")
    git("-c","user.name=Test","-c","user.email=test@example.invalid","commit","-qm","feature")
    env = os.environ.copy()
    env["LOCALAPPDATA"]=str(path/"profile")
    env["XDG_DATA_HOME"]=str(path/"profile")
    env.pop("CODEGYM_PROVIDER",None)
    env.pop("CODEGYM_MODEL",None)
    env.pop("CODEGYM_API_KEY_ENV",None)
    run = subprocess.run([str(BINARY),
         "feature","--repo",str(repo),"--base","main","--questions","1",
         "--provider","openai-compatible","--model","mock-english",
         "--base-url",f"http://127.0.0.1:{server.server_port}/v1"],
         input="It prevents negative values.\n",text=True,capture_output=True,
         encoding="utf-8",errors="replace",env=env,timeout=35)
    print("EXIT:",run.returncode)
    print("RESPONSE:\n",run.stdout[-4500:])
    print("REQUESTS:",len(Mock.requests))
    assert run.returncode==0,(run.stderr,run.stdout)
    assert "Why does this function clamp values at zero?" in run.stdout
    assert "Correct reasoning." in run.stdout
    assert "average score" in run.stdout
    assert len(Mock.requests)==2
    assert all(not re.search(r"[\u0400-\u052f]",s) for s in Mock.requests)
    assert not re.search(r"[\u0400-\u052f]",run.stdout)
server.shutdown()
print("ENGLISH_E2E_PASS")
