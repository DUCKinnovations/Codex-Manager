"""
http_register_v2.py — 纯 HTTP 批量注册 ChatGPT (curl_cffi + LanU app.ashx)
  - curl_cffi TLS 指纹伪装
  - LanU app.ashx API 收验证码
  - 两阶段: 先 20 线程激活邮箱 (mailGetLast)，再 20 线程注册+OTP
  - token 批次保存 + OTP 数据库同步 + 去重
"""

import json
import os
import queue
import re
import sys
import time
import random
import string
import secrets
import hashlib
import base64
import sqlite3
import threading
import argparse
from datetime import datetime, timezone
from typing import Any, Optional
import urllib.parse
import urllib.request
import urllib.error
from curl_cffi import requests

try:
    sys.stdout.reconfigure(encoding="utf-8")
    sys.stderr.reconfigure(encoding="utf-8")
except Exception:
    pass

SCRIPT_DIR = os.path.dirname(os.path.abspath(__file__))
CONFIG_PATH = os.path.join(SCRIPT_DIR, "config.json")

def load_config():
    with open(CONFIG_PATH, "r", encoding="utf-8") as f:
        return json.load(f)

cfg = load_config()
PROXY = cfg.get("proxy", "") or ""
RESIDENTIAL_PROXIES = cfg.get("residential_proxies", [])
# 蓝邮建议直连，避免代理对 1443 CONNECT 的限制
LANU_DIRECT = bool(cfg.get("lanu_direct", True))
OTP_POLL_MAX_WAIT = int(cfg.get("otp_poll_max_wait", 120))
OTP_POLL_INTERVAL = int(cfg.get("otp_poll_interval", 3))
# 默认执行策略：单线程 + 单账号（可用命令行覆盖）
DEFAULT_THREADS = int(cfg.get("default_threads", 1))
DEFAULT_COUNT = int(cfg.get("default_count", 1))
DONE_FROM_DB = bool(cfg.get("done_from_db", False))

RESULT_FILE = os.path.join(SCRIPT_DIR, "lanu_results.json")
TOKEN_DIR = os.path.join(SCRIPT_DIR, "tokens")
BATCH_SIZE = 10
# DB_PATH = r"C:\Users\21120\Desktop\otp_service.db"
DB_PATH = os.path.join(os.path.expanduser("~"), "Desktop", "otp_service.db")

AUTH_URL = "https://auth.openai.com/oauth/authorize"
TOKEN_URL = "https://auth.openai.com/oauth/token"
CLIENT_ID = "app_EMoamEEZ73f0CkXaXp7hrann"
REDIRECT_URI = "http://localhost:1455/auth/callback"
SCOPE = "openid email profile offline_access"

LANU_API = "https://mail5.lanu.cn:1443/app.ashx"

# Clash API 自动切换节点（线程安全 IP 池，两轮内不重复）
CLASH_API = "http://127.0.0.1:9090"
CLASH_SECRET = "123456"
CLASH_GROUP = "节点选择"

MAX_DELAY = 500

DELAY_REFRESH_INTERVAL = 10

class ClashPool:
    def __init__(self):
        self._lock = threading.Lock()
        self._nodes = []
        self._queue = []
        self._recent = []
        self._all_candidates = []
        self._stop_event = threading.Event()

    def _headers(self):
        return {"Authorization": f"Bearer {CLASH_SECRET}", "Content-Type": "application/json"}

    def _fetch_good_nodes(self, verbose=True):
        """获取当前低延迟且存活的节点列表"""
        import urllib.request
        req = urllib.request.Request(
            f"{CLASH_API}/proxies",
            headers=self._headers()
        )
        all_data = json.loads(urllib.request.urlopen(req, timeout=5).read())

        req2 = urllib.request.Request(
            f"{CLASH_API}/proxies/{urllib.parse.quote(CLASH_GROUP)}",
            headers=self._headers()
        )
        group_data = json.loads(urllib.request.urlopen(req2, timeout=5).read())

        skip = {"DIRECT", "REJECT", "REJECT-DROP", "COMPATIBLE", "PASS", "GLOBAL",
                "节点选择", "自动选择", "故障转移"}
        candidates = [n for n in group_data.get("all", [])
                      if n not in skip and "剩余" not in n and "到期" not in n
                      and "重置" not in n and "超时" not in n and "订阅" not in n]
        self._all_candidates = candidates

        good = []
        for name in candidates:
            proxy = all_data.get("proxies", {}).get(name, {})
            if not proxy.get("alive", False):
                continue
            hist = proxy.get("history", [])
            delay = hist[-1].get("delay", 0) if hist else 0
            if 0 < delay <= MAX_DELAY:
                good.append((name, delay))

        good.sort(key=lambda x: x[1])
        return good

    def load(self):
        try:
            good = self._fetch_good_nodes(verbose=True)
            with self._lock:
                self._nodes = [n for n, _ in good]
                random.shuffle(self._nodes)
                self._queue = list(self._nodes)
                self._recent = []
            print(f"  Clash IP 池: {len(self._nodes)} 个低延迟节点 (≤{MAX_DELAY}ms)")
            if good:
                delays = [d for _, d in good]
                print(f"  延迟范围: {min(delays)}ms ~ {max(delays)}ms")
        except Exception as e:
            print(f"  Clash API 加载失败: {e}")
            self._nodes = []

    def _refresh_loop(self):
        """后台每 DELAY_REFRESH_INTERVAL 秒刷新节点延迟"""
        while not self._stop_event.is_set():
            self._stop_event.wait(DELAY_REFRESH_INTERVAL)
            if self._stop_event.is_set():
                break
            try:
                good = self._fetch_good_nodes(verbose=False)
                new_nodes = [n for n, _ in good]
                with self._lock:
                    old_set = set(self._nodes)
                    new_set = set(new_nodes)
                    added = new_set - old_set
                    removed = old_set - new_set
                    self._nodes = new_nodes
                    self._queue = [n for n in self._queue if n in new_set]
                    for n in added:
                        if n not in self._recent:
                            self._queue.append(n)
                    random.shuffle(self._queue)
                if added or removed:
                    print(f"  [Clash 刷新] 可用: {len(new_nodes)} 个"
                          f"{f', +{len(added)}' if added else ''}"
                          f"{f', -{len(removed)}' if removed else ''}")
            except Exception:
                pass

    def start_refresh(self):
        """启动后台延迟刷新线程"""
        self._stop_event.clear()
        t = threading.Thread(target=self._refresh_loop, daemon=True)
        t.start()
        print(f"  Clash 延迟刷新: 每 {DELAY_REFRESH_INTERVAL}s")

    def stop_refresh(self):
        self._stop_event.set()

    def acquire(self):
        """线程安全地取一个节点，两轮内不重复"""
        with self._lock:
            if not self._nodes:
                return None
            if not self._queue:
                self._queue = [n for n in self._nodes if n not in self._recent]
                if not self._queue:
                    self._recent.clear()
                    self._queue = list(self._nodes)
                random.shuffle(self._queue)
            node = self._queue.pop(0)
            self._recent.append(node)
            max_recent = len(self._nodes) * 2
            if len(self._recent) > max_recent:
                self._recent = self._recent[-len(self._nodes):]
            return node

    def switch_to(self, node):
        """通过 Clash API 切换到指定节点"""
        if not node:
            return
        try:
            import urllib.request
            body = json.dumps({"name": node}).encode()
            req = urllib.request.Request(
                f"{CLASH_API}/proxies/{urllib.parse.quote(CLASH_GROUP)}",
                data=body, headers=self._headers(), method="PUT"
            )
            urllib.request.urlopen(req, timeout=5)
        except Exception as e:
            print(f"  Clash 切换失败: {e}")

_clash_pool = ClashPool()

# ═══════════════════════════════════════════════
# 线程安全
# ═══════════════════════════════════════════════
_print_lock = threading.Lock()
_file_lock = threading.Lock()
_counter_lock = threading.Lock()
_counter = {"ok": 0, "fail": 0}
_shared_token_batch = []
_save_queue: queue.Queue = queue.Queue()
_saver_stop = threading.Event()


def tprint(tid: int, msg: str):
    with _print_lock:
        print(f"  [T{tid}] {msg}")


# ═══════════════════════════════════════════════
# 加载邮箱 & 去重
# ═══════════════════════════════════════════════
def load_accounts(filepath: str) -> list:
    accounts = []
    with open(filepath, "r", encoding="utf-8-sig") as f:
        for line in f:
            line = line.strip()
            if not line or "@" not in line or line.startswith("#"):
                continue
            parts = line.split("----")
            if len(parts) >= 2:
                accounts.append({
                    "email": parts[0].strip(),
                    "password": parts[1].strip(),
                    "mail_url": parts[2].strip() if len(parts) >= 3 else "",
                })
    return accounts


def parse_api_token(mail_url: str) -> str:
    if not mail_url:
        return ""
    parts = mail_url.rstrip("/").split("/")
    return parts[-1] if len(parts) >= 2 else ""


def load_done_emails() -> set:
    done = set()
    if os.path.exists(RESULT_FILE):
        try:
            with open(RESULT_FILE, "r", encoding="utf-8") as f:
                data = json.load(f)
            done |= {r["email"].lower() for r in data if r.get("success") or r.get("banned")}
        except Exception:
            pass
    if DONE_FROM_DB and os.path.exists(DB_PATH):
        try:
            conn = sqlite3.connect(DB_PATH)
            cur = conn.cursor()
            cur.execute("SELECT mailbox_email FROM accounts")
            done |= {row[0].lower() for row in cur.fetchall() if row[0]}
            conn.close()
        except Exception:
            pass
    return done


# ═══════════════════════════════════════════════
# LanU app.ashx API
# ═══════════════════════════════════════════════
def lanu_get_latest(tid: int, email: str, api_token: str,
                    proxies: Any, silent: bool = False) -> Optional[dict]:
    try:
        lanu_proxies = None if LANU_DIRECT else proxies
        r = requests.get(LANU_API,
                         params={"act": "mailGetLast", "email": email, "s": api_token},
                         impersonate="chrome", proxies=lanu_proxies, timeout=15, verify=False)
        if r.status_code != 200:
            if not silent:
                tprint(tid, f"[LanU] HTTP {r.status_code}")
            return None
        text = (r.text or "").strip()
        if not text:
            return None
        try:
            data = r.json()
        except Exception:
            if not silent:
                tprint(tid, f"[LanU] 非JSON: {text[:120]}")
            return None
        if isinstance(data, dict) and data.get("code") not in (0, None):
            msg = str(data.get("msg", ""))
            if not (silent and data.get("code") == 2 and "没有新邮件" in msg):
                if data.get("code") == 2 and "余额" in msg:
                    tprint(tid, "[LanU] 余额不足")
                elif not silent:
                    tprint(tid, f"[LanU] code={data.get('code')} msg={msg}")
        return data
    except Exception as e:
        if not silent:
            tprint(tid, f"[LanU] 异常: {e}")
    return None


def extract_mail_parts(data: dict) -> tuple:
    subj = body = sender = ""
    inner = data.get("data")
    if isinstance(inner, dict):
        subj = str(inner.get("Title") or inner.get("Subject") or "")
        body = str(inner.get("Body") or inner.get("body") or inner.get("content") or "")
        sender = str(inner.get("FromAddress") or inner.get("FromAddr") or "").lower()
    if not subj:
        rows = data.get("rows") or data.get("list") or []
        if rows and isinstance(rows, list):
            item = rows[0]
            subj = str(item.get("Subject") or item.get("Title") or "")
            body = str(item.get("Body") or item.get("body") or "")
            sender = str(item.get("FromAddr") or item.get("From") or "").lower()
    if not subj:
        subj = str(data.get("Subject") or data.get("Title") or "")
    if not body:
        body = str(data.get("Body") or data.get("body") or data.get("content") or "")
    if not sender:
        sender = str(data.get("FromAddr") or data.get("FromAddress") or "").lower()
    return subj, body, sender


def lanu_poll_otp_worker(tid: int, email: str, api_token: str,
                         seen: set, result_q: queue.Queue,
                         max_wait: int = 120, poll_interval: int = 3,
                         proxies: Any = None):
    regex = r"(?<!\d)(\d{6})(?!\d)"
    seen_local = set(seen)
    t0 = time.time()
    last_status_time = t0
    poll_count = 0
    while time.time() - t0 < max_wait:
        try:
            data = lanu_get_latest(tid, email, api_token, proxies, silent=True)
            poll_count += 1
            now = time.time()
            if now - last_status_time >= 30:
                elapsed = int(now - t0)
                api_code = data.get("code", "?") if data else "null"
                api_msg = str(data.get("msg", ""))[:40] if data else "无响应"
                tprint(tid, f"[LanU] 轮询中 {elapsed}s/{max_wait}s 第{poll_count}次 api_code={api_code} {api_msg}")
                last_status_time = now
            if data and data.get("code") == 0:
                subj, body, sender = extract_mail_parts(data)
                full = subj + " " + body
                is_new = subj and subj not in seen_local
                is_otp = ("openai" in sender or "openai" in full.lower()
                          or "verify" in full.lower()
                          or "verification" in subj.lower()
                          or "code" in subj.lower())
                if is_new and (is_otp or re.search(regex, full)):
                    seen_local.add(subj)
                    m = re.search(regex, full)
                    if m:
                        tprint(tid, f"[LanU] 验证码: {m.group(1)} ({int(now-t0)}s)")
                        result_q.put(m.group(1))
                        return
        except Exception:
            pass
        time.sleep(poll_interval)
    tprint(tid, f"[LanU] 轮询超时 {max_wait}s 共{poll_count}次")
    result_q.put("")


def wait_otp_once(
    tid: int,
    email: str,
    api_token: str,
    proxies: Any,
    send_otp_fn=None,
) -> str:
    seen_subjects = set()
    pre = lanu_get_latest(tid, email, api_token, proxies, silent=True)
    if pre and pre.get("code") == 0:
        subj, _, _ = extract_mail_parts(pre)
        if subj:
            seen_subjects.add(subj)

    tprint(tid, "[LanU] 快照完成, 启动轮询")
    otp_q = queue.Queue()
    poller = threading.Thread(
        target=lanu_poll_otp_worker,
        args=(tid, email, api_token, seen_subjects, otp_q),
        kwargs={"max_wait": OTP_POLL_MAX_WAIT,
                "poll_interval": OTP_POLL_INTERVAL,
                "proxies": proxies},
        daemon=True,
    )
    poller.start()

    if send_otp_fn is not None:
        try:
            send_resp = send_otp_fn()
            code = getattr(send_resp, "status_code", "ERR")
            tprint(tid, f"Send OTP: {code}")
            if code != 200:
                return ""
        except Exception as e:
            tprint(tid, f"Send OTP 异常: {e}")
            return ""

    try:
        code = otp_q.get(timeout=max(OTP_POLL_MAX_WAIT + 10, 45))
    except queue.Empty:
        code = ""
    return code


# ═══════════════════════════════════════════════
# OAuth / PKCE
# ═══════════════════════════════════════════════
def _b64url(raw: bytes) -> str:
    return base64.urlsafe_b64encode(raw).decode("ascii").rstrip("=")


def _decode_b64url_json(seg: str) -> Optional[dict]:
    try:
        seg += "=" * ((4 - len(seg) % 4) % 4)
        data = json.loads(base64.urlsafe_b64decode(seg))
        return data if isinstance(data, dict) else None
    except Exception:
        return None

def generate_oauth() -> dict:
    state = secrets.token_urlsafe(16)
    verifier = secrets.token_urlsafe(64)
    challenge = _b64url(hashlib.sha256(verifier.encode("ascii")).digest())
    params = {
        "client_id": CLIENT_ID, "response_type": "code",
        "redirect_uri": REDIRECT_URI, "scope": SCOPE,
        "state": state, "code_challenge": challenge,
        "code_challenge_method": "S256", "prompt": "login",
        "id_token_add_organizations": "true",
        "codex_cli_simplified_flow": "true",
    }
    return {"url": f"{AUTH_URL}?{urllib.parse.urlencode(params)}",
            "state": state, "verifier": verifier}


def exchange_token(callback_url: str, verifier: str, expected_state: str) -> dict:
    parsed = urllib.parse.urlparse(callback_url)
    qs = urllib.parse.parse_qs(parsed.query, keep_blank_values=True)
    frag = urllib.parse.parse_qs(parsed.fragment, keep_blank_values=True)
    for k, v in frag.items():
        qs.setdefault(k, v)
    code = (qs.get("code", [""])[0] or "").strip()
    state = (qs.get("state", [""])[0] or "").strip()
    if not code:
        raise ValueError("callback 缺少 code")
    if state != expected_state:
        raise ValueError("state 不匹配")
    body = urllib.parse.urlencode({
        "grant_type": "authorization_code", "client_id": CLIENT_ID,
        "code": code, "redirect_uri": REDIRECT_URI, "code_verifier": verifier,
    })
    headers = {
        "Content-Type": "application/x-www-form-urlencoded", "Accept": "application/json",
    }
    r = requests.post(
        TOKEN_URL,
        data=body,
        headers=headers,
        timeout=30,
        impersonate="chrome",
    )
    if r.status_code != 200:
        raise RuntimeError(f"oauth/token 失败: {r.status_code} {r.text[:200]}")
    data = r.json()
    return data


def try_exchange_from_oauth_redirects(s: requests.Session, oauth: dict, tid: int,
                                      max_hops: int = 8) -> Optional[dict]:
    """兜底：登录态建立后，重新走 OAuth 重定向链拿 code/state。"""
    cur = oauth["url"]
    for _ in range(max_hops):
        try:
            r = s.get(cur, allow_redirects=False, timeout=15)
        except Exception as e:
            tprint(tid, f"OAuth重定向链异常: {e}")
            return None
        loc = r.headers.get("Location", "")
        if r.status_code in (301, 302, 303, 307, 308) and loc:
            nxt = urllib.parse.urljoin(cur, loc)
            if "code=" in nxt and "state=" in nxt:
                try:
                    return exchange_token(nxt, oauth["verifier"], oauth["state"])
                except Exception as e:
                    tprint(tid, f"OAuth换token失败: {e}")
                    return None
            cur = nxt
            continue
        break
    # 有些流程最终会落到 200 页面，URL 中才带 code/state
    try:
        r = s.get(cur, allow_redirects=True, timeout=20)
        final_url = getattr(r, "url", "") or ""
        if "code=" in final_url and "state=" in final_url:
            return exchange_token(final_url, oauth["verifier"], oauth["state"])
    except Exception:
        pass
    return None


def try_exchange_from_start_url(s: requests.Session, start_url: str, oauth: dict, tid: int,
                                max_hops: int = 8) -> Optional[dict]:
    """从指定 URL 开始走重定向，尝试捕获 callback 里的 code/state。"""
    if not start_url:
        return None
    cur = (urllib.parse.urljoin("https://auth.openai.com", start_url)
           if "://" not in start_url else start_url)
    for _ in range(max_hops):
        try:
            r = s.get(cur, allow_redirects=False, timeout=15)
        except Exception as e:
            tprint(tid, f"重定向链异常: {e}")
            return None
        loc = r.headers.get("Location", "")
        if r.status_code in (301, 302, 303, 307, 308) and loc:
            nxt = urllib.parse.urljoin(cur, loc)
            if "code=" in nxt and "state=" in nxt:
                try:
                    return exchange_token(nxt, oauth["verifier"], oauth["state"])
                except Exception as e:
                    tprint(tid, f"重定向换token失败: {e}")
                    return None
            cur = nxt
            continue
        break
    # 兼容：最终 200 页面 URL 本身带 code/state
    try:
        r = s.get(cur, allow_redirects=True, timeout=20)
        final_url = getattr(r, "url", "") or ""
        if "code=" in final_url and "state=" in final_url:
            return exchange_token(final_url, oauth["verifier"], oauth["state"])
    except Exception:
        pass
    return None


def get_sentinel(did: str, proxies: Any, imp: str) -> str:
    r = requests.post(
        "https://sentinel.openai.com/backend-api/sentinel/req",
        headers={
            "origin": "https://sentinel.openai.com",
            "referer": "https://sentinel.openai.com/backend-api/sentinel/frame.html?sv=20260219f9f6",
            "content-type": "text/plain;charset=UTF-8",
        },
        data=json.dumps({"p": "", "id": did, "flow": "authorize_continue"}),
        proxies=proxies, impersonate=imp, timeout=15,
    )
    if r.status_code != 200:
        raise RuntimeError(f"Sentinel 失败: {r.status_code}")
    t = r.json()["token"]
    return json.dumps({"p": "", "t": "", "c": t, "id": did, "flow": "authorize_continue"})


# ═══════════════════════════════════════════════
# 工具
# ═══════════════════════════════════════════════
NAMES_F = ["Alex", "Chris", "Jordan", "Taylor", "Morgan", "Sam", "Casey",
           "Jamie", "Riley", "Quinn", "Michael", "Emily"]
NAMES_L = ["Smith", "Johnson", "Williams", "Brown", "Jones", "Garcia",
           "Miller", "Davis", "Wilson", "Moore", "Taylor", "Anderson"]

def gen_name():
    return f"{random.choice(NAMES_F)} {random.choice(NAMES_L)}"

def gen_birthday():
    return f"{random.randint(1980, 2002)}-{random.randint(1, 9):02d}-{random.randint(10, 28)}"


# ═══════════════════════════════════════════════
# 核心注册流程 (照搬 mail_register 的 register_one)
# ═══════════════════════════════════════════════
def register_one(tid: int, acct: dict, proxies: Any) -> Optional[dict]:
    email = acct["email"]
    mail_pwd = acct["password"]
    chatgpt_pwd = mail_pwd
    name = gen_name()
    birthday = gen_birthday()

    imp = random.choice(["chrome", "chrome110", "chrome116", "edge"])
    s = requests.Session(proxies=proxies, impersonate=imp)

    try:
        trace = s.get("https://cloudflare.com/cdn-cgi/trace", timeout=10).text
        loc = (re.search(r"^loc=(.+)$", trace, re.MULTILINE) or
               type("", (), {"group": lambda s, n: "??"})()).group(1)
        tprint(tid, f"IP={loc}  指纹={imp}")

        oauth = generate_oauth()
        s.get(oauth["url"], timeout=15)
        did = s.cookies.get("oai-did")

        tprint(tid, f"邮箱: {email}")
        sentinel = get_sentinel(did, proxies, imp)
        signup_resp = s.post(
            "https://auth.openai.com/api/accounts/authorize/continue",
            headers={"referer": "https://auth.openai.com/create-account",
                     "accept": "application/json",
                     "content-type": "application/json",
                     "openai-sentinel-token": sentinel},
            data=json.dumps({"username": {"value": email, "kind": "email"},
                             "screen_hint": "signup"}),
        )
        if signup_resp.status_code in (403, 429):
            tprint(tid, f"被拒: {signup_resp.status_code}")
            return None
        page_type = signup_resp.json().get("page", {}).get("type", "")

        api_token = parse_api_token(acct.get("mail_url", ""))
        if not api_token:
            tprint(tid, "无 API 令牌, 跳过")
            return None

        account_exists = False
        need_otp = False
        post_otp_start_url = ""
        post_otp_page_type = ""

        if page_type == "create_account_password":
            tprint(tid, f"注册密码: {chatgpt_pwd}")
            reg = s.post(
                "https://auth.openai.com/api/accounts/user/register",
                headers={"referer": "https://auth.openai.com/create-account/password",
                         "accept": "application/json",
                         "content-type": "application/json"},
                data=json.dumps({"password": chatgpt_pwd, "username": email}),
            )
            if reg.status_code == 200:
                nxt = reg.json().get("page", {}).get("type", "")
                tprint(tid, f"Register OK -> {nxt}")
                if "otp" in nxt or "email" in nxt:
                    need_otp = True
            else:
                account_exists = True
                tprint(tid, "账号已存在 -> 登录流程")
        elif page_type == "login_password":
            account_exists = True
            tprint(tid, "账号已存在 -> 登录流程")
        else:
            tprint(tid, f"未知页面: {page_type}")
            return None

        # 登录 OTP 流程
        if account_exists:
            def _send_login_otp():
                sentinel2 = get_sentinel(did, proxies, imp)
                s.post("https://auth.openai.com/api/accounts/authorize/continue",
                       headers={"referer": "https://auth.openai.com/log-in",
                                "accept": "application/json",
                                "content-type": "application/json",
                                "openai-sentinel-token": sentinel2},
                       data=json.dumps({"username": {"value": email, "kind": "email"},
                                        "screen_hint": "login"}))
                return s.post(
                    "https://auth.openai.com/api/accounts/passwordless/send-otp",
                    headers={"referer": "https://auth.openai.com/log-in/passwordless",
                             "accept": "application/json",
                             "content-type": "application/json"})

            code = wait_otp_once(
                tid=tid,
                email=email,
                api_token=api_token,
                proxies=proxies,
                send_otp_fn=_send_login_otp,
            )
            if not code:
                tprint(tid, "验证码超时")
                return None
            need_otp = True

            val = s.post("https://auth.openai.com/api/accounts/email-otp/validate",
                         headers={"referer": "https://auth.openai.com/email-verification",
                                  "accept": "application/json",
                                  "content-type": "application/json"},
                         data=json.dumps({"code": code}))
            tprint(tid, f"OTP验证: {val.status_code}")
            if val.status_code != 200:
                tprint(tid, f"OTP拒绝: {val.text[:200]}")
                if "deleted or deactivated" in (val.text or ""):
                    tprint(tid, f"账号已被封禁，标记跳过")
                    return {"_banned": True}
                return None
            try:
                vj = val.json() or {}
                pg = (vj.get("page") or {})
                page_type = pg.get("type", "") if isinstance(pg, dict) else ""
                post_otp_page_type = page_type or post_otp_page_type
                post_otp_start_url = (vj.get("continue_url") or pg.get("continue_url") or
                                      pg.get("url") or "")
                if page_type:
                    tprint(tid, f"OTP后页面: {page_type}")
                if post_otp_start_url:
                    tprint(tid, "OTP后拿到继续URL")
            except Exception:
                pass

        # 新账号 OTP 流程
        if need_otp and not account_exists:
            def _send_signup_otp():
                # 新注册流程优先尝试 email-otp/send，失败时回退 passwordless/send-otp
                last = None
                candidates = [
                    ("https://auth.openai.com/api/accounts/email-otp/send",
                     "https://auth.openai.com/email-verification"),
                    ("https://auth.openai.com/api/accounts/passwordless/send-otp",
                     "https://auth.openai.com/log-in/passwordless"),
                ]
                for url, referer in candidates:
                    resp = s.post(
                        url,
                        headers={"referer": referer,
                                 "accept": "application/json",
                                 "content-type": "application/json"},
                        data="{}",
                    )
                    last = resp
                    if resp.status_code == 200:
                        return resp
                return last

            code = wait_otp_once(
                tid=tid,
                email=email,
                api_token=api_token,
                proxies=proxies,
                send_otp_fn=_send_signup_otp,
            )
            if not code:
                tprint(tid, "验证码超时")
                return None
            val = s.post("https://auth.openai.com/api/accounts/email-otp/validate",
                         headers={"referer": "https://auth.openai.com/email-verification",
                                  "accept": "application/json",
                                  "content-type": "application/json"},
                         data=json.dumps({"code": code}))
            tprint(tid, f"OTP验证: {val.status_code}")
            if val.status_code != 200:
                tprint(tid, f"OTP拒绝: {val.text[:200]}")
                if "deleted or deactivated" in (val.text or ""):
                    tprint(tid, f"账号已被封禁，标记跳过")
                    return {"_banned": True}
                return None
            try:
                vj = val.json() or {}
                pg = (vj.get("page") or {})
                page_type = pg.get("type", "") if isinstance(pg, dict) else ""
                post_otp_page_type = page_type or post_otp_page_type
                post_otp_start_url = (vj.get("continue_url") or pg.get("continue_url") or
                                      pg.get("url") or "")
                if page_type:
                    tprint(tid, f"OTP后页面: {page_type}")
                if post_otp_start_url:
                    tprint(tid, "OTP后拿到继续URL")
            except Exception:
                pass

        # 需要补全资料时（含 about_you 页面）提交资料
        need_profile = (not account_exists) or (post_otp_page_type == "about_you")
        if need_profile:
            tprint(tid, f"创建: {name}, {birthday}")
            create_resp = s.post("https://auth.openai.com/api/accounts/create_account",
                                 headers={"referer": "https://auth.openai.com/about-you",
                                          "accept": "application/json",
                                          "content-type": "application/json"},
                                 data=json.dumps({"name": name, "birthdate": birthday}))
            tprint(tid, f"create_account: {create_resp.status_code}")
            if create_resp.status_code != 200:
                txt = (create_resp.text or "").strip()
                if txt:
                    tprint(tid, f"create_account响应: {txt[:240]}")
            try:
                cj = create_resp.json() or {}
                cpg = (cj.get("page") or {})
                ctype = cpg.get("type", "") if isinstance(cpg, dict) else ""
                curl = (cj.get("continue_url") or cpg.get("continue_url")
                        or cpg.get("url") or "")
                if ctype:
                    tprint(tid, f"创建后页面: {ctype}")
                if curl:
                    post_otp_start_url = curl
                    tprint(tid, "创建后拿到继续URL")
            except Exception:
                pass

        # 获取 Token: 优先 workspace 链路，失败时回退 OAuth 重定向链路
        auth_cookie = s.cookies.get("oai-client-auth-session")
        if auth_cookie:
            parts = auth_cookie.split(".")
            # 常见 JWT: [header, payload, sign]，优先解 payload(第2段)；
            # 兼容非标准格式，失败时回退尝试第1段。
            candidates = []
            if len(parts) >= 2:
                candidates.append(parts[1])
            candidates.append(parts[0])
            auth_json = None
            for seg in candidates:
                obj = _decode_b64url_json(seg)
                if obj:
                    auth_json = obj
                    break

            if auth_json:
                workspaces = auth_json.get("workspaces") or []
                if workspaces:
                    wid = (workspaces[0] or {}).get("id", "")
                    sel = s.post("https://auth.openai.com/api/accounts/workspace/select",
                                 headers={"referer": "https://auth.openai.com/sign-in-with-chatgpt/codex/consent",
                                          "content-type": "application/json"},
                                 data=json.dumps({"workspace_id": wid}))
                    if sel.status_code == 200:
                        cont = (sel.json() or {}).get("continue_url", "")
                        if cont:
                            cur = cont
                            for _ in range(6):
                                r = s.get(cur, allow_redirects=False, timeout=15)
                                loc = r.headers.get("Location", "")
                                if r.status_code not in (301, 302, 303, 307, 308) or not loc:
                                    break
                                nxt = urllib.parse.urljoin(cur, loc)
                                if "code=" in nxt and "state=" in nxt:
                                    td = exchange_token(nxt, oauth["verifier"], oauth["state"])
                                    tprint(tid, f"成功! {email}")
                                    return {
                                        "email": email, "mail_pwd": mail_pwd,
                                        "chatgpt_pwd": chatgpt_pwd,
                                        "tokens": td,
                                    }
                                cur = nxt
                    else:
                        tprint(tid, f"workspace/select: {sel.status_code}")
                else:
                    tprint(tid, "无 workspace，尝试 OAuth 兜底链路")
            else:
                tprint(tid, "授权 Cookie 解析失败，尝试 OAuth 兜底链路")
        else:
            tprint(tid, "无授权 Cookie，尝试 OAuth 兜底链路")

        # 兜底A：优先从 OTP 返回的继续URL推进
        td = None
        if post_otp_start_url:
            td = try_exchange_from_start_url(s, post_otp_start_url, oauth, tid)
            if td and td.get("access_token"):
                tprint(tid, f"成功(OTP继续URL)! {email}")
                return {
                    "email": email, "mail_pwd": mail_pwd,
                    "chatgpt_pwd": chatgpt_pwd,
                    "tokens": td,
                }

        # 兜底B：重新走 OAuth 重定向链，拿 code/state
        td = try_exchange_from_oauth_redirects(s, oauth, tid)
        if td and td.get("access_token"):
            tprint(tid, f"成功(兜底)! {email}")
            return {
                "email": email, "mail_pwd": mail_pwd,
                "chatgpt_pwd": chatgpt_pwd,
                "tokens": td,
            }

        tprint(tid, "重定向链中未找到 callback")
        return None
    except Exception as e:
        tprint(tid, f"异常: {e}")
        return None


# ═══════════════════════════════════════════════
# 保存: lanu_results.json + OTP DB + token 批次
# ═══════════════════════════════════════════════
def upsert_db(email: str, mail_pwd: str):
    now = datetime.now(timezone.utc).isoformat()
    with _file_lock:
        conn = sqlite3.connect(DB_PATH)
        cur = conn.cursor()
        cur.execute("""CREATE TABLE IF NOT EXISTS accounts (
            account_id TEXT PRIMARY KEY, openai_email TEXT,
            mailbox_email TEXT, mailbox_password TEXT,
            mailtm_token TEXT DEFAULT '', token_updated_at TEXT,
            created_at TEXT, updated_at TEXT)""")
        aid = email.lower()
        cur.execute("SELECT account_id FROM accounts WHERE account_id=?", (aid,))
        if cur.fetchone():
            cur.execute("UPDATE accounts SET openai_email=?,mailbox_email=?,mailbox_password=?,updated_at=? WHERE account_id=?",
                        (email, email, mail_pwd, now, aid))
        else:
            cur.execute("INSERT INTO accounts (account_id,openai_email,mailbox_email,mailbox_password,created_at,updated_at) VALUES (?,?,?,?,?,?)",
                        (aid, email, email, mail_pwd, now, now))
        conn.commit()
        conn.close()


def save_result(email, mail_pwd, chatgpt_pwd, token_data, success, banned=False):
    record = {
        "email": email, "mail_password": mail_pwd,
        "openai_password": chatgpt_pwd, "success": success,
        "tokens": token_data or {},
        "time": datetime.now().isoformat(timespec="seconds"),
    }
    if banned:
        record["banned"] = True
    with _file_lock:
        records = []
        if os.path.exists(RESULT_FILE):
            try:
                with open(RESULT_FILE, "r", encoding="utf-8") as f:
                    records = json.load(f)
            except Exception:
                pass
        records = [r for r in records if r.get("email", "").lower() != email.lower()]
        records.append(record)
        with open(RESULT_FILE, "w", encoding="utf-8") as f:
            json.dump(records, f, indent=2, ensure_ascii=False)


def _next_batch_number():
    os.makedirs(TOKEN_DIR, exist_ok=True)
    max_num = 56
    for name in os.listdir(TOKEN_DIR):
        m = re.match(r"^tokens_(\d+)\.json$", name)
        if m:
            max_num = max(max_num, int(m.group(1)))
    return max_num + 1


def append_token_and_flush(item: dict):
    with _file_lock:
        _shared_token_batch.append(item)
        while len(_shared_token_batch) >= BATCH_SIZE:
            chunk = _shared_token_batch[:BATCH_SIZE]
            del _shared_token_batch[:BATCH_SIZE]
            os.makedirs(TOKEN_DIR, exist_ok=True)
            num = _next_batch_number()
            fname = f"tokens_{num:03d}.json"
            path = os.path.join(TOKEN_DIR, fname)
            with open(path, "w", encoding="utf-8") as f:
                json.dump(chunk, f, indent=2, ensure_ascii=False)
            print(f"  Token ({len(chunk)}个) -> {fname}")


def flush_remaining():
    with _file_lock:
        if _shared_token_batch:
            os.makedirs(TOKEN_DIR, exist_ok=True)
            num = _next_batch_number()
            fname = f"tokens_{num:03d}.json"
            path = os.path.join(TOKEN_DIR, fname)
            with open(path, "w", encoding="utf-8") as f:
                json.dump(list(_shared_token_batch), f, indent=2, ensure_ascii=False)
            print(f"  尾部 Token ({len(_shared_token_batch)}个) -> {fname}")
            _shared_token_batch.clear()


# ═══════════════════════════════════════════════
# 激活邮箱：先调一次 LanU API，再进入注册/OTP
# ═══════════════════════════════════════════════
def activate_mailbox(tid: int, acct: dict, proxies: Any) -> bool:
    """激活邮箱：调用一次 mailGetLast，便于后续 OTP 轮询稳定"""
    email = acct.get("email", "")
    api_token = parse_api_token(acct.get("mail_url", ""))
    if not api_token:
        return False
    try:
        data = lanu_get_latest(tid, email, api_token, proxies, silent=True)
        if data is not None:
            return True
    except Exception:
        pass
    return False


def run_activation_phase(todo: list, threads: int, proxies: Any, use_residential: bool) -> None:
    """Phase 1: 用 threads 个线程先激活所有待注册账号的邮箱"""
    if not todo:
        return
    print(f"\n  Phase 1: 激活邮箱 ({len(todo)} 个, {threads} 线程)")
    sem = threading.Semaphore(threads)
    done = 0
    lock = threading.Lock()

    def _activate_one(tid: int, acct: dict, idx: int):
        nonlocal done
        if use_residential and RESIDENTIAL_PROXIES:
            rp = RESIDENTIAL_PROXIES[(tid - 1) % len(RESIDENTIAL_PROXIES)]
            px = {"http": rp, "https": rp}
        else:
            px = proxies or {}
        with sem:
            ok = activate_mailbox(tid, acct, px)
            with lock:
                done += 1
            if ok:
                tprint(tid, f"[激活] [{idx}/{len(todo)}] {acct['email'][:20]}... OK")
            else:
                tprint(tid, f"[激活] [{idx}/{len(todo)}] {acct['email'][:20]}... 跳过(无token或蓝邮请求失败)")
            time.sleep(random.uniform(0.3, 0.8))

    ths = []
    for i, acct in enumerate(todo):
        tid = (i % threads) + 1
        t = threading.Thread(target=_activate_one, args=(tid, acct, i + 1), daemon=True)
        ths.append(t)
        t.start()
        time.sleep(random.uniform(0.05, 0.15))
    for t in ths:
        t.join()
    print(f"  Phase 1 完成: 已激活 {done}/{len(todo)} 个邮箱\n")


# ═══════════════════════════════════════════════
# Worker & 主入口
# ═══════════════════════════════════════════════
_clash_switch_lock = threading.Lock()

def test_openai_proxy(proxy_url: str, timeout: int = 10) -> tuple[bool, str]:
    px = {"http": proxy_url, "https": proxy_url}
    try:
        r = requests.get("https://cloudflare.com/cdn-cgi/trace",
                         proxies=px, impersonate="chrome", timeout=timeout)
        if r.status_code != 200:
            return False, f"HTTP {r.status_code}"
        loc = (re.search(r"^loc=(.+)$", r.text or "", re.MULTILINE) or
               type("", (), {"group": lambda s, n: "??"})()).group(1)
        return True, f"loc={loc}"
    except Exception as e:
        return False, str(e)


def worker(tid: int, acct: dict, idx: int, total: int, proxies: Any, use_residential: bool):
    email = acct["email"]

    if use_residential and RESIDENTIAL_PROXIES:
        rp = RESIDENTIAL_PROXIES[(tid - 1) % len(RESIDENTIAL_PROXIES)]
        proxies = {"http": rp, "https": rp}
        from urllib.parse import urlparse
        parsed = urlparse(rp)
        sid = ""
        if parsed.username and "sid-" in parsed.username:
            sid = parsed.username.split("sid-")[1].split("-")[0]
        tprint(tid, f"住宅IP sid={sid}")

    tprint(tid, f"━━ [{idx}/{total}] {email} ━━")
    result = register_one(tid, acct, proxies)
    if result and result.get("_banned"):
        with _counter_lock:
            _counter["fail"] += 1
        save_result(email, acct["password"], "", None, False, banned=True)
        tprint(tid, f"已标记封禁: {email}")
    elif result:
        with _counter_lock:
            _counter["ok"] += 1
        td = result["tokens"]
        save_result(email, result["mail_pwd"], result["chatgpt_pwd"], td, True)
        try:
            upsert_db(email, result["mail_pwd"])
            tprint(tid, f"DB 写入: {email}")
        except Exception as e:
            tprint(tid, f"DB 异常: {e}")
        append_token_and_flush({
            "email": email,
            "tokens": {
                "access_token": td.get("access_token", ""),
                "id_token": td.get("id_token", ""),
                "refresh_token": td.get("refresh_token", ""),
            }
        })
    else:
        with _counter_lock:
            _counter["fail"] += 1
        save_result(email, acct["password"], "", None, False)
    time.sleep(random.uniform(1, 3))


def refresh_one_token(rt: str, proxies: Any) -> Optional[dict]:
    """用 refresh_token 刷新获取新的 access_token / id_token / refresh_token"""
    try:
        imp = random.choice(["chrome", "chrome110", "chrome116", "edge"])
        r = requests.post(
            TOKEN_URL,
            headers={"Content-Type": "application/x-www-form-urlencoded",
                     "Accept": "application/json"},
            data=urllib.parse.urlencode({
                "grant_type": "refresh_token",
                "client_id": CLIENT_ID,
                "refresh_token": rt,
            }),
            proxies=proxies, impersonate=imp, timeout=20,
        )
        if r.status_code == 200:
            return r.json()
        return None
    except Exception:
        return None


def run_refresh(refresh_files: list, threads: int):
    """刷新指定的 token 文件"""
    proxies = {"http": PROXY, "https": PROXY} if PROXY else None

    files = []
    for f in refresh_files:
        p = f if os.path.isabs(f) else os.path.join(TOKEN_DIR, f)
        if not os.path.isfile(p):
            print(f"  文件不存在: {p}")
            continue
        files.append(p)

    if not files:
        print("  无有效文件")
        return

    total = 0
    ok = 0
    fail = 0
    lock = threading.Lock()
    file_data = {}
    for path in files:
        try:
            with open(path, "r", encoding="utf-8") as f:
                file_data[path] = json.load(f)
            total += len(file_data[path])
        except Exception as e:
            print(f"  读取 {path} 失败: {e}")
            file_data[path] = []

    print(f"\n{'='*60}")
    print(f"  批量刷新 Token (refresh_token → 新 access_token)")
    print(f"  文件: {len(files)} 个, 账号: {total} 个, 并发: {threads}")
    print(f"  代理: {PROXY or '直连'}")
    print(f"{'='*60}\n")

    if total == 0:
        print("  无 token 可刷新")
        return

    sem = threading.Semaphore(threads)

    def _refresh_item(fname, idx_in_file, item):
        nonlocal ok, fail
        email = item.get("email", "?")
        rt = item.get("tokens", {}).get("refresh_token", "")
        if not rt:
            with lock:
                fail += 1
            print(f"  [{email}] 无 refresh_token, 跳过")
            return

        with sem:
            td = refresh_one_token(rt, proxies)

        if td and td.get("access_token"):
            new_tokens = {
                "access_token": td.get("access_token", ""),
                "id_token": td.get("id_token", ""),
                "refresh_token": td.get("refresh_token", rt),
            }
            with lock:
                file_data[fname][idx_in_file]["tokens"] = new_tokens
                ok += 1
            print(f"  [{email}] 刷新成功")
        else:
            with lock:
                fail += 1
            print(f"  [{email}] 刷新失败")

    threads_list = []
    for path in files:
        items = file_data[path]
        for idx, item in enumerate(items):
            t = threading.Thread(target=_refresh_item, args=(path, idx, item), daemon=True)
            threads_list.append(t)
            t.start()
            time.sleep(random.uniform(0.1, 0.3))

    for t in threads_list:
        t.join()

    for path, data in file_data.items():
        try:
            with open(path, "w", encoding="utf-8") as f:
                json.dump(data, f, indent=2, ensure_ascii=False)
            print(f"  已覆写: {os.path.basename(path)}")
        except Exception as e:
            print(f"  写入 {path} 失败: {e}")

    print(f"\n{'='*60}")
    print(f"  刷新完成! 成功: {ok}  失败: {fail}  总计: {total}")
    print(f"{'='*60}")


def main():
    parser = argparse.ArgumentParser(description="纯HTTP批量注册 ChatGPT (curl_cffi + LanU)")
    parser.add_argument("--file", default="")
    parser.add_argument("--threads", type=int, default=DEFAULT_THREADS)
    parser.add_argument("--start", type=int, default=0)
    parser.add_argument("--count", type=int, default=DEFAULT_COUNT)
    parser.add_argument("--refresh", nargs="+", metavar="FILE",
                        help="刷新模式: 指定 token 文件, 如 tokens_056.json tokens_057.json")
    parser.add_argument("--residential", action="store_true",
                        help="使用住宅代理注册 (config.json 中的 residential_proxies)")
    parser.add_argument("--no-activate", action="store_true",
                        help="跳过 Phase 1 激活邮箱，直接注册+OTP")
    parser.add_argument("--skip-proxy-check", action="store_true",
                        help="跳过启动前代理预检（可能在 Phase 2 中途失败）")
    args = parser.parse_args()

    if args.refresh:
        run_refresh(args.refresh, args.threads)
        return

    acct_file = args.file
    if not acct_file:
        candidates = sorted(
            [n for n in os.listdir(SCRIPT_DIR)
             if n.endswith(".txt") and "mailtb" in n.lower()],
        )
        if not candidates:
            print("  未找到 mailtb 账号文件")
            sys.exit(1)
        acct_file = os.path.join(SCRIPT_DIR, candidates[-1])

    accounts = load_accounts(acct_file)
    done = load_done_emails()
    todo = [a for a in accounts if a["email"].lower() not in done]

    if args.count > 0:
        todo = todo[args.start:args.start + args.count]
    elif args.start > 0:
        todo = todo[args.start:]

    if args.residential:
        if not RESIDENTIAL_PROXIES:
            print("  未配置 residential_proxies，无法使用 --residential")
            sys.exit(1)
        proxies = {"http": RESIDENTIAL_PROXIES[0], "https": RESIDENTIAL_PROXIES[0]}
        proxy_info = f"住宅代理 x{len(RESIDENTIAL_PROXIES)} (随机轮换)"
    else:
        proxies = {"http": PROXY, "https": PROXY} if PROXY else None
        proxy_info = PROXY or "直连"

    print(f"\n{'='*60}")
    print(f"  纯 HTTP 批量注册 ChatGPT (curl_cffi + LanU app.ashx)")
    print(f"  账号文件: {os.path.basename(acct_file)}")
    print(f"  总账号: {len(accounts)}, 已完成: {len(accounts)-len(todo)}, 待注册: {len(todo)}")
    print(f"  并发: {args.threads}")
    print(f"  代理: {proxy_info}")
    print(f"  蓝邮链路: {'直连' if LANU_DIRECT else '跟随代理'}")
    print(f"  OTP轮询: wait={OTP_POLL_MAX_WAIT}s interval={OTP_POLL_INTERVAL}s")
    print(f"  已完成判定(数据库): {'启用' if DONE_FROM_DB else '关闭'}")
    print(f"  邮件API: {LANU_API}")
    print(f"  Token目录: {TOKEN_DIR}")
    print(f"{'='*60}\n")

    if not todo:
        if not os.path.exists(RESULT_FILE):
            try:
                with open(RESULT_FILE, "w", encoding="utf-8") as f:
                    json.dump([], f, indent=2, ensure_ascii=False)
                print(f"  已初始化结果文件: {RESULT_FILE}")
            except Exception as e:
                print(f"  初始化结果文件失败: {e}")
        print("  全部已完成!")
        return

    # 启动前探测 OpenAI 链路，避免 Phase 2 全部超时/中断
    if args.skip_proxy_check:
        print("  已跳过代理预检 (--skip-proxy-check)")
    else:
        if args.residential and RESIDENTIAL_PROXIES:
            print("  预检住宅代理链路 (OpenAI)...")
            ok_probe = False
            max_probe = min(len(RESIDENTIAL_PROXIES), max(args.threads, 8))
            for i, rp in enumerate(RESIDENTIAL_PROXIES[:max_probe], start=1):
                ok, detail = test_openai_proxy(rp, timeout=8)
                if ok:
                    print(f"  住宅代理可用 [{i}/{max_probe}]: {detail}")
                    ok_probe = True
                    break
                if i <= 3:
                    print(f"  住宅代理失败 [{i}/{max_probe}]: {detail[:120]}")
            if not ok_probe:
                print("  住宅代理预检失败：未找到可用节点，请更换代理后重试")
                sys.exit(2)
        elif proxies and isinstance(proxies, dict):
            purl = proxies.get("https") or proxies.get("http")
            if purl:
                print("  预检代理链路 (OpenAI)...")
                ok, detail = test_openai_proxy(purl, timeout=8)
                if ok:
                    print(f"  代理可用: {detail}")
                else:
                    print(f"  代理预检失败: {detail[:200]}")
                    print("  请先修复代理，或清空 config.json 的 proxy 后走直连测试")
                    sys.exit(2)

    # Phase 1: 先激活邮箱，再 Phase 2 注册+OTP
    if not args.no_activate:
        run_activation_phase(todo, args.threads, proxies, args.residential)
    print(f"  Phase 2: 注册 + OTP ({args.threads} 线程)\n")

    total = len(todo)
    sem = threading.Semaphore(args.threads)
    threads = []

    def _run(tid, acct, idx):
        with sem:
            worker(tid, acct, idx, total, proxies, args.residential)
            delay = random.uniform(5, 8)
            tprint(tid, f"等待 {delay:.0f}s 再继续...")
            time.sleep(delay)

    for i, acct in enumerate(todo):
        tid = (i % args.threads) + 1
        t = threading.Thread(target=_run, args=(tid, acct, i + 1), daemon=True)
        threads.append(t)
        t.start()
        time.sleep(random.uniform(3, 8))

    try:
        for t in threads:
            t.join()
    except KeyboardInterrupt:
        print("\n  用户中断")
    finally:
        flush_remaining()

    print(f"\n{'='*60}")
    print(f"  完成! 成功: {_counter['ok']}  失败: {_counter['fail']}  总计: {total}")
    print(f"  结果: {RESULT_FILE}")
    print(f"  Token: {TOKEN_DIR}")
    print(f"{'='*60}")


if __name__ == "__main__":
    main()
