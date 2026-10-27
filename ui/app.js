(() => {
  const isTauri = !!(window.__TAURI__ && window.__TAURI__.core);
  document.body.classList.add(isTauri ? "desktop" : "web");

  const $ = (id) => document.getElementById(id);
  const stage = $("stage");
  const layer = $("layer");
  const mask = $("mask");

  function fit() {
    const s = Math.min(window.innerWidth / 720, window.innerHeight / 450);
    document.documentElement.style.setProperty("--scale", String(Math.max(0.25, s)));
  }
  fit();
  window.addEventListener("resize", fit);

  function apiBase() {
    const path = location.pathname;
    const idx = path.indexOf("/launcher/");
    return idx > 0 ? path.slice(0, idx) : "";
  }
  const PREVIEW_KEY = "mclauncher-rust-preview";
  let state = null;
  let remote = [];
  let filter = "release";
  let pickedRemote = "";
  let progressHandler = null;

  function blankState() {
    return {
      config: {
        gameDir: "",
        javaPath: "",
        maxMemoryMb: 1024,
        memoryAuto: true,
        downloadSource: "bmclapi",
        debug: false,
        versionIsolation: false,
        selectedVersion: "",
        clientToken: "",
        activeAccount: "",
        accounts: [],
        setupDone: true,
      },
      installed: [],
      javaLabel: "",
    };
  }

  function loadPreview() {
    try {
      const raw = localStorage.getItem(PREVIEW_KEY);
      if (raw) return JSON.parse(raw);
    } catch (_) {}
    const s = blankState();
    s.config.gameDir = ".minecraft";
    return s;
  }
  function savePreview() {
    localStorage.setItem(PREVIEW_KEY, JSON.stringify(state));
  }

  const api = {
    async state() {
      if (isTauri) return window.__TAURI__.core.invoke("app_state");
      state = state || loadPreview();
      return state;
    },
    async save(patch) {
      if (isTauri) return window.__TAURI__.core.invoke("save_settings", { patch });
      Object.assign(state.config, patch);
      savePreview();
      return state;
    },
    async offline(name) {
      if (isTauri) return window.__TAURI__.core.invoke("offline_login", { name });
      name = (name || "").trim();
      if (!name) throw new Error("没有填写游戏名字");
      if ([...name].length > 16) throw new Error("游戏名字不能超过 16 个字符");
      const acc = {
        id: crypto.randomUUID(),
        kind: "offline",
        name,
        uuid: "preview",
        accessToken: "0",
        userType: "legacy",
        server: null,
      };
      state.config.accounts = state.config.accounts.filter((a) => !(a.kind === "offline" && a.name === name));
      state.config.accounts.push(acc);
      state.config.activeAccount = acc.id;
      savePreview();
      return state;
    },
    async yggdrasil(server, username, password) {
      if (isTauri) return window.__TAURI__.core.invoke("yggdrasil_login", { server, username, password });
      const res = await fetch(apiBase() + "/api/mc/yggdrasil", {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ server, username, password }),
      });
      const data = await res.json();
      if (!res.ok) throw new Error(data.error || "登录失败");
      const acc = data.account;
      state.config.accounts = state.config.accounts.filter((a) => a.uuid !== acc.uuid || a.server !== acc.server);
      state.config.accounts.push(acc);
      state.config.activeAccount = acc.id;
      savePreview();
      return state;
    },
    async removeAccount(id) {
      if (isTauri) return window.__TAURI__.core.invoke("remove_account", { id });
      state.config.accounts = state.config.accounts.filter((a) => a.id !== id);
      if (state.config.activeAccount === id) state.config.activeAccount = state.config.accounts[0]?.id || "";
      savePreview();
      return state;
    },
    async useAccount(id) {
      if (isTauri) return window.__TAURI__.core.invoke("use_account", { id });
      state.config.activeAccount = id;
      savePreview();
      return state;
    },
    async versions() {
      if (isTauri) return window.__TAURI__.core.invoke("list_versions");
      const source = state.config.downloadSource || "bmclapi";
      const res = await fetch(apiBase() + "/api/mc/manifest?source=" + encodeURIComponent(source));
      const data = await res.json();
      if (!res.ok) throw new Error(data.error || "获取列表失败");
      return data.versions.map((v) => ({ ...v, installed: state.installed.includes(v.id) }));
    },
    async install(id) {
      if (isTauri) return window.__TAURI__.core.invoke("install_version", { id });
      const item = remote.find((v) => v.id === id);
      if (!item) throw new Error("没有这个版本");
      const res = await fetch(apiBase() + "/api/mc/version?url=" + encodeURIComponent(item.url));
      const data = await res.json();
      if (!res.ok) throw new Error(data.error || "下载版本信息失败");
      if (!state.installed.includes(id)) state.installed.unshift(id);
      state.config.selectedVersion = id;
      savePreview();
      return { preview: true, id, mainClass: data.mainClass || "" };
    },
    async cancel() {
      if (isTauri) return window.__TAURI__.core.invoke("cancel_install");
    },
    async launch() {
      if (isTauri) return window.__TAURI__.core.invoke("launch_game");
      throw new Error("预览环境不能启动 Minecraft。用 GitHub Actions 构建出的桌面版会调用本机 Java。");
    },
    async findJava() {
      if (isTauri) return window.__TAURI__.core.invoke("find_java");
      return { path: state.config.javaPath || "", major: 0, found: !!state.config.javaPath };
    },
    async openDir() {
      if (isTauri) return window.__TAURI__.core.invoke("open_game_dir");
      await msg("info", "提示", "游戏目录：\n" + (state.config.gameDir || ".minecraft") + "\n\n桌面版会用系统文件管理器打开这个文件夹。", ["确定"]);
    },
    async deleteVersion(id) {
      if (isTauri) return window.__TAURI__.core.invoke("delete_version", { id });
      state.installed = state.installed.filter((v) => v !== id);
      if (state.config.selectedVersion === id) state.config.selectedVersion = state.installed[0] || "";
      savePreview();
      return state;
    },
    async renameVersion(id, name) {
      if (isTauri) return window.__TAURI__.core.invoke("rename_version", { id, name });
      if (!name || name.includes("/") || name.includes("\\")) throw new Error("版本号不合法");
      state.installed = state.installed.map((v) => (v === id ? name : v));
      if (state.config.selectedVersion === id) state.config.selectedVersion = name;
      savePreview();
      return state;
    },
  };

  if (isTauri) {
    window.__TAURI__.event.listen("download-progress", (ev) => {
      if (progressHandler) progressHandler(ev.payload);
    });
  }

  function activeAccount() {
    return (state.config.accounts || []).find((a) => a.id === state.config.activeAccount) || null;
  }

  function renderMain() {
    const id = state.config.selectedVersion;
    $("verlabel").textContent = id || "未安装";
    const list = $("verlist");
    list.innerHTML = "";
    const ids = state.installed || [];
    if (!ids.length) {
      const b = document.createElement("button");
      b.type = "button";
      b.textContent = "（没有已安装的版本）";
      b.disabled = true;
      list.appendChild(b);
    }
    ids.forEach((ver) => {
      const b = document.createElement("button");
      b.type = "button";
      b.textContent = ver;
      if (ver === id) b.className = "on";
      b.onclick = async (e) => {
        e.stopPropagation();
        state = await api.save({ selectedVersion: ver });
        $("verlist").classList.add("hidden");
        renderMain();
      };
      list.appendChild(b);
    });
  }

  function openLayer(node) {
    mask.classList.remove("hidden");
    layer.appendChild(node);
  }
  function closeTop() {
    if (layer.lastElementChild) layer.lastElementChild.remove();
    if (!layer.children.length) mask.classList.add("hidden");
  }
  function closeLayer() {
    layer.innerHTML = "";
    mask.classList.add("hidden");
  }

  function msg(kind, title, text, buttons) {
    return new Promise((resolve) => {
      const win = document.createElement("div");
      win.className = "win small";
      const icons = { error: "error", warn: "warn", ask: "ask", info: "info", ok: "ok" };
      win.innerHTML = `
        <div class="win-title"><span></span><button type="button" data-x>×</button></div>
        <div class="win-body">
          <div class="msg">
            <img alt="" />
            <div><p></p><div class="msg-actions"></div></div>
          </div>
        </div>`;
      win.querySelector(".win-title span").textContent = title;
      win.querySelector("img").src = "assets/dlg/" + (icons[kind] || "info") + ".png";
      win.querySelector("p").textContent = text;
      const actions = win.querySelector(".msg-actions");
      buttons.forEach((label, i) => {
        const b = document.createElement("button");
        b.type = "button";
        b.textContent = label;
        b.onclick = () => {
          closeTop();
          resolve(label);
        };
        if (i === 0) b.style.minWidth = "72px";
        actions.appendChild(b);
      });
      win.querySelector("[data-x]").onclick = () => {
        closeTop();
        resolve(buttons[buttons.length - 1] || "取消");
      };
      openLayer(win);
    });
  }

  async function promptName() {
    return new Promise((resolve) => {
      const win = document.createElement("div");
      win.className = "win small";
      win.innerHTML = `
        <div class="win-title"><span>提示</span><button type="button" data-x>×</button></div>
        <div class="win-body">
          <div class="msg"><img src="assets/dlg/ask.png" alt="" /><div>
            <p>没有填写游戏名字</p>
            <input id="pname" type="text" maxlength="16" placeholder="玩家名，建议英文数字下划线" />
            <div class="msg-actions"><button type="button" data-ok>确定</button><button type="button" data-no>取消</button></div>
          </div></div>
        </div>`;
      const done = (v) => {
        closeTop();
        resolve(v);
      };
      win.querySelector("[data-ok]").onclick = () => done(win.querySelector("#pname").value);
      win.querySelector("[data-no]").onclick = () => done(null);
      win.querySelector("[data-x]").onclick = () => done(null);
      openLayer(win);
      setTimeout(() => win.querySelector("#pname").focus(), 30);
    });
  }

  async function ensureAccount() {
    if (activeAccount()) return true;
    const name = await promptName();
    if (!name) return false;
    try {
      state = await api.offline(name);
      return true;
    } catch (e) {
      await msg("error", "错误", String(e.message || e), ["确定"]);
      return false;
    }
  }

  $("verbtn").onclick = (e) => {
    e.stopPropagation();
    $("verlist").classList.toggle("hidden");
  };
  stage.addEventListener("click", () => $("verlist").classList.add("hidden"));
  $("close").onclick = async () => {
    if (isTauri) window.__TAURI__.window.getCurrentWindow().close();
    else await msg("info", "提示", "这是网页预览。桌面版点这里会关闭窗口。", ["确定"]);
  };
  $("opendir").onclick = () => api.openDir().catch((e) => msg("error", "错误", String(e.message || e), ["确定"]));

  $("launch").onclick = async () => {
    if (!state.config.selectedVersion) {
      await msg("warn", "警告", "没有选择游戏,请选择或点击更新按钮更新游戏", ["确定"]);
      return;
    }
    if (!(await ensureAccount())) return;
    if (isTauri) {
      const java = await api.findJava();
      if (!java.found && !state.config.javaPath) {
        await openJavaSetup();
        const again = await api.findJava();
        if (!again.found && !state.config.javaPath) return;
      }
    }
    try {
      const pid = await api.launch();
      await msg("ok", "提示", "游戏已启动" + (pid ? "\nPID " + pid : ""), ["确定"]);
    } catch (e) {
      await msg("error", "错误", String(e.message || e), ["确定"]);
    }
  };

  $("update").onclick = () => openDownload().catch((e) => msg("error", "错误", String(e.message || e), ["确定"]));
  $("settings").onclick = () => openSettings();

  function sourceName() {
    return state.config.downloadSource === "official" ? "官方" : "BMCLAPI";
  }

  async function openDownload() {
    const win = document.createElement("div");
    win.className = "win";
    win.innerHTML = `
      <div class="win-title"><span>下载中心</span><button type="button" data-x>×</button></div>
      <div class="win-body">
        <div class="tabs">
          <button type="button" class="on" data-tab="vanilla">官方原版</button>
          <button type="button" disabled title="后续版本再做">Forge</button>
          <button type="button" disabled title="后续版本再做">Forge+OptiFine</button>
          <button type="button" disabled title="后续版本再做">简单整合</button>
          <button type="button" disabled title="安装版本时会一起下载声音和语言文件">更新声音文件</button>
        </div>
        <div class="filters">
          <label class="chk"><input type="radio" name="flt" value="release" checked /> 正式版</label>
          <label class="chk"><input type="radio" name="flt" value="snapshot" /> 快照预览版</label>
          <label class="chk"><input type="radio" name="flt" value="all" /> 全部</label>
          <input class="search" id="q" type="text" placeholder="搜索版本" />
          <span class="grow"></span>
          <span id="srcnote"></span>
        </div>
        <div class="table-wrap">
          <table>
            <thead><tr><th style="width:120px">版本</th><th style="width:110px">名称</th><th>简介</th></tr></thead>
            <tbody id="tbody"></tbody>
          </table>
        </div>
        <div class="row">
          <span id="dlstatus" class="note">正在获取列表…</span>
          <span class="bar">
            <button type="button" id="install">安装选中版本</button>
            <button type="button" data-x>关闭</button>
          </span>
        </div>
      </div>`;
    win.querySelector("#srcnote").textContent = "下载源：" + sourceName();
    win.querySelectorAll("[data-x]").forEach((b) => (b.onclick = closeLayer));
    win.querySelectorAll('input[name="flt"]').forEach((r) => {
      r.checked = r.value === filter;
      r.onchange = () => {
        filter = r.value;
        paint();
      };
    });
    win.querySelector("#q").oninput = paint;
    win.querySelector("#install").onclick = () => startInstall(pickedRemote);
    openLayer(win);
    try {
      remote = await api.versions();
      win.querySelector("#dlstatus").textContent = "共 " + remote.length + " 个版本";
      paint();
    } catch (e) {
      win.querySelector("#dlstatus").textContent = "";
      await msg("error", "错误", String(e.message || e), ["确定"]);
    }

    function paint() {
      const q = (win.querySelector("#q").value || "").trim().toLowerCase();
      const body = win.querySelector("#tbody");
      body.innerHTML = "";
      remote
        .filter((v) => filter === "all" || v.typeName === filter)
        .filter((v) => !q || v.id.toLowerCase().includes(q))
        .slice(0, 400)
        .forEach((v) => {
          const tr = document.createElement("tr");
          if (v.id === pickedRemote) tr.className = "sel";
          const time = (v.releaseTime || "").replace("T", " ").replace("Z", "");
          tr.innerHTML = "<td></td><td></td><td></td>";
          tr.children[0].textContent = v.id + (v.installed ? "  已安装" : "");
          tr.children[1].textContent = v.typeLabel || v.typeName;
          tr.children[2].textContent = time;
          tr.onclick = () => {
            pickedRemote = v.id;
            paint();
          };
          tr.ondblclick = () => startInstall(v.id);
          body.appendChild(tr);
        });
    }
  }

  async function startInstall(id) {
    if (!id) {
      await msg("warn", "警告", "请在更新声音资源按钮旁的列表里选择想要更新的游戏版本", ["确定"]);
      return;
    }
    closeLayer();
    const win = document.createElement("div");
    win.className = "win small";
    win.innerHTML = `
      <div class="win-title"><span>正在更新</span><span></span></div>
      <div class="win-body">
        <div class="grid2">
          <span>文件名:</span><span id="pf" style="grid-column: span 2"></span>
          <span>文件大小:</span><span id="ps" style="grid-column: span 2"></span>
          <span>下载速度:</span><span id="pv" style="grid-column: span 2"></span>
        </div>
        <div class="progress"><span id="pbar"></span></div>
        <div class="note" id="pm"></div>
        <div class="msg-actions"><button type="button" id="pcancel">取消</button></div>
      </div>`;
    openLayer(win);
    const bar = win.querySelector("#pbar");
    const set = (p) => {
      win.querySelector("#pf").textContent = p.file || id;
      const total = p.bytesTotal || 0;
      win.querySelector("#ps").textContent = total ? fmt(p.bytes || 0) + " / " + fmt(total) : fmt(p.bytes || 0);
      win.querySelector("#pv").textContent = (p.speed ? fmt(p.speed) + "/s" : "0KB/S") + "    " + (p.completed || 0) + "/" + (p.total || 0);
      const ratio = p.total ? Math.min(100, Math.round(((p.completed || 0) / p.total) * 100)) : 0;
      bar.style.width = ratio + "%";
      if (p.message) win.querySelector("#pm").textContent = p.message;
    };
    progressHandler = set;
    win.querySelector("#pcancel").onclick = async () => {
      await api.cancel();
      progressHandler = null;
      closeLayer();
    };
    if (!isTauri) {
      set({ file: id + ".json", bytes: 0, bytesTotal: 0, speed: 0, completed: 0, total: 1, message: "正在下载版本信息" });
    }
    try {
      const result = await api.install(id);
      progressHandler = null;
      state = await api.state();
      renderMain();
      closeLayer();
      const extra = result && result.preview ? "\n预览只确认了版本清单。库、资源和客户端会在桌面版下载。" : "";
      await msg("ok", "完成", "版本 " + id + " 已准备好。" + extra, ["确定"]);
    } catch (e) {
      progressHandler = null;
      closeLayer();
      if (String(e.message || e) !== "已取消") await msg("error", "错误", String(e.message || e), ["确定"]);
    }
  }

  function fmt(n) {
    if (!n) return "0 B";
    if (n < 1024) return n + " B";
    if (n < 1024 * 1024) return (n / 1024).toFixed(1) + " KB";
    return (n / 1024 / 1024).toFixed(2) + " MB";
  }

  function openSettings() {
    const c = state.config;
    const win = document.createElement("div");
    win.className = "win";
    win.innerHTML = `
      <div class="win-title"><span>游戏设置</span><button type="button" data-x>×</button></div>
      <div class="win-body"><div class="form" id="form"></div></div>`;
    win.querySelector("[data-x]").onclick = closeLayer;
    const form = win.querySelector("#form");
    form.innerHTML = `
      <div class="group">
        <h3>游戏管理</h3>
        <div class="row"><span>游戏列表</span><span id="gcount" class="note"></span></div>
        <div class="accounts" id="glist"></div>
        <div class="row">
          <button type="button" id="gdel">删除选中游戏</button>
          <button type="button" id="gren">重命名</button>
          <span class="grow"></span>
        </div>
        <div class="note">游戏目录(Forge mods,材质包,存档的目录)</div>
        <div class="row"><input id="gdir" type="text" /><button type="button" id="gopen">打开目录</button></div>
        <label class="chk"><input id="iso0" type="radio" name="iso" /> 保存到.minecraft</label>
        <label class="chk"><input id="iso1" type="radio" name="iso" /> 保存到.minecraft\\versions\\版本名称\\</label>
        <div class="note">只在1.6以上的版本有效</div>
      </div>
      <div class="group">
        <h3>登录</h3>
        <div class="accounts" id="alist"></div>
        <div class="row"><span>离线登录</span><span class="note">无需联网，仅保存玩家名称</span></div>
        <div class="row"><input id="offname" type="text" placeholder="玩家名" maxlength="16" /><button type="button" id="offok">登录</button></div>
        <div class="row"><span>外置登录</span><span class="note">authlib-injector / Yggdrasil</span></div>
        <input id="yserver" type="text" placeholder="认证服务器，例如 https://littleskin.cn/api/yggdrasil" />
        <div class="row">
          <input id="yuser" type="text" placeholder="邮箱或角色名" />
          <input id="ypass" type="password" placeholder="密码" />
          <button type="button" id="yok">登录</button>
        </div>
        <div class="note">外置登录不会保存密码，只保存令牌。微软登录即将支持。</div>
        <button type="button" disabled>微软登录（即将支持）</button>
      </div>
      <div class="group">
        <h3>Java 与内存</h3>
        <div class="grid2">
          <span>java路径</span>
          <input id="jpath" type="text" />
          <button type="button" id="jfind">查找java</button>
        </div>
        <label class="chk"><input id="mauto" type="checkbox" /> 自动设置（最大 1300M）</label>
        <div class="row"><span>最大内存</span><input id="mem" type="number" min="256" max="32768" style="width:100px" /><span>MB</span><span class="note">手动</span></div>
        <div class="note">自动设置将根据原版习惯使用最大 1300M。如需要更大的内存,请手动设置。</div>
        <label class="chk"><input id="debug" type="checkbox" /> 调试模式（生成 输出日志.txt）</label>
      </div>
      <div class="group">
        <h3>下载源</h3>
        <label class="chk"><input type="radio" name="src" value="official" /> 官方</label>
        <label class="chk"><input type="radio" name="src" value="bmclapi" /> BMCLAPI</label>
        <div class="note">版本清单、库、资源和客户端都会走选中的源。BMCLAPI 适合国内网络。</div>
      </div>
      <div class="row"><span></span><button type="button" id="save">保存设置</button></div>`;
    form.querySelector("#gdir").value = c.gameDir || "";
    form.querySelector("#jpath").value = c.javaPath || "";
    form.querySelector("#mem").value = c.maxMemoryMb || 1024;
    form.querySelector("#mauto").checked = !!c.memoryAuto;
    form.querySelector("#debug").checked = !!c.debug;
    form.querySelector(c.versionIsolation ? "#iso1" : "#iso0").checked = true;
    form.querySelector(`input[name="src"][value="${c.downloadSource === "official" ? "official" : "bmclapi"}"]`).checked = true;
    form.querySelector("#gcount").textContent = (state.installed || []).length + " 个已安装";
    const glist = form.querySelector("#glist");
    (state.installed || []).forEach((id) => {
      const b = document.createElement("button");
      b.type = "button";
      b.textContent = id;
      if (id === c.selectedVersion) b.className = "on";
      b.onclick = () => {
        glist.querySelectorAll("button").forEach((x) => x.classList.remove("on"));
        b.classList.add("on");
        glist.dataset.sel = id;
      };
      glist.appendChild(b);
    });
    const alist = form.querySelector("#alist");
    (c.accounts || []).forEach((a) => {
      const b = document.createElement("button");
      b.type = "button";
      const kind = a.kind === "yggdrasil" ? "外置" : "离线";
      b.textContent = `${a.name}  ·  ${kind}` + (a.server ? "  ·  " + a.server : "");
      if (a.id === c.activeAccount) b.className = "on";
      b.onclick = async () => {
        state = await api.useAccount(a.id);
        closeLayer();
        openSettings();
      };
      b.ondblclick = async () => {
        state = await api.removeAccount(a.id);
        closeLayer();
        openSettings();
      };
      alist.appendChild(b);
    });
    if (!c.accounts?.length) {
      const b = document.createElement("button");
      b.type = "button";
      b.disabled = true;
      b.textContent = "还没有账号。双击已有账号可删除。";
      alist.appendChild(b);
    }
    form.querySelector("#gopen").onclick = () => api.openDir();
    form.querySelector("#jfind").onclick = async () => {
      try {
        const j = await api.findJava();
        if (j.found && j.path) {
          form.querySelector("#jpath").value = j.path;
          await msg("ok", "提示", "找到java路径,已经自动填入\n" + j.path + (j.major ? "\nJava " + j.major : ""), ["确定"]);
        } else await msg("warn", "警告", "没有找到java路径,请手动指定javaw.exe", ["确定"]);
      } catch (e) {
        await msg("error", "错误", String(e.message || e), ["确定"]);
      }
    };
    form.querySelector("#offok").onclick = async () => {
      try {
        state = await api.offline(form.querySelector("#offname").value);
        await msg("ok", "提示", "离线登录成功", ["确定"]);
        closeLayer();
        openSettings();
      } catch (e) {
        await msg("error", "错误", String(e.message || e), ["确定"]);
      }
    };
    form.querySelector("#yok").onclick = async () => {
      try {
        state = await api.yggdrasil(
          form.querySelector("#yserver").value,
          form.querySelector("#yuser").value,
          form.querySelector("#ypass").value,
        );
        await msg("ok", "提示", "登录成功", ["确定"]);
        closeLayer();
        openSettings();
      } catch (e) {
        await msg("error", "登录失败", String(e.message || e), ["确定"]);
      }
    };
    form.querySelector("#gdel").onclick = async () => {
      const id = glist.dataset.sel;
      if (!id) return;
      const ok = await msg("ask", "提示", "你确认要删除此版本游戏吗\n将会删除 .minecraft\\versions\\" + id, ["确定", "取消"]);
      if (ok !== "确定") return;
      state = await api.deleteVersion(id);
      renderMain();
      closeLayer();
      openSettings();
    };
    form.querySelector("#gren").onclick = async () => {
      const id = glist.dataset.sel;
      if (!id) return;
      const name = await promptName();
      if (!name) return;
      try {
        state = await api.renameVersion(id, name);
        renderMain();
        closeLayer();
        openSettings();
      } catch (e) {
        await msg("error", "错误", String(e.message || e), ["确定"]);
      }
    };
    form.querySelector("#save").onclick = async () => {
      const src = form.querySelector('input[name="src"]:checked').value;
      state = await api.save({
        gameDir: form.querySelector("#gdir").value.trim(),
        javaPath: form.querySelector("#jpath").value.trim(),
        maxMemoryMb: Number(form.querySelector("#mem").value) || 1024,
        memoryAuto: form.querySelector("#mauto").checked,
        debug: form.querySelector("#debug").checked,
        versionIsolation: form.querySelector("#iso1").checked,
        downloadSource: src,
      });
      await msg("ok", "提示", "保存设置", ["确定"]);
      closeLayer();
      renderMain();
    };
    openLayer(win);
  }

  function openJavaSetup() {
    return new Promise((resolve) => {
      const win = document.createElement("div");
      win.className = "win small";
      win.innerHTML = `
        <div class="win-title"><span>请完成以下设置</span><button type="button" data-x>×</button></div>
        <div class="win-body">
          <p class="note">没有找到java路径,请手动指定javaw.exe</p>
          <div class="grid2"><span>最大内存</span><input id="mm" type="number" value="1024" /><span>MB</span></div>
          <div class="grid2"><span>java路径</span><input id="jp" type="text" placeholder="javaw.exe" /><button type="button" id="jf">查找java</button></div>
          <div class="msg-actions"><button type="button" id="ok">完成设置</button><button type="button" data-x>取消</button></div>
        </div>`;
      const done = async (save) => {
        if (save) {
          state = await api.save({
            javaPath: win.querySelector("#jp").value.trim(),
            maxMemoryMb: Number(win.querySelector("#mm").value) || 1024,
            memoryAuto: false,
            setupDone: true,
          });
        }
        closeLayer();
        resolve();
      };
      win.querySelector("#ok").onclick = () => done(true);
      win.querySelectorAll("[data-x]").forEach((b) => (b.onclick = () => done(false)));
      win.querySelector("#jf").onclick = async () => {
        const j = await api.findJava();
        if (j.path) win.querySelector("#jp").value = j.path;
        else await msg("warn", "警告", "没有找到java路径,请手动指定javaw.exe", ["确定"]);
      };
      openLayer(win);
    });
  }

  api.state().then((s) => {
    state = s;
    renderMain();
  }).catch((e) => {
    state = blankState();
    renderMain();
    msg("error", "错误", String(e.message || e), ["确定"]);
  });
})();
