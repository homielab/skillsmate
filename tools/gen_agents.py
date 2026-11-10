#!/usr/bin/env python3
"""Regenerate crates/skills-core/src/agents/table.rs from the upstream CLI.

Usage: python3 tools/gen_agents.py <skills-repo>/src/agents.ts <commit> > crates/skills-core/src/agents/table.rs

Detection rules that are not a plain directory check are listed in SPECIAL;
review new upstream agents by hand when the script fails on one.
"""
import re,sys
src=open(sys.argv[1]).read()
commit=sys.argv[2]
body=src[src.index('export const agents'):src.index('export async function detectInstalledAgents')]
ENV={'claudeHome':('CLAUDE_CONFIG_DIR','.claude'),'autohandHome':('AUTOHAND_HOME','.autohand'),
     'grokHome':('GROK_HOME','.grok'),'hermesHome':('HERMES_HOME','.hermes'),'vibeHome':('VIBE_HOME','.vibe'),
     'codexHome':('CODEX_HOME','.codex'),'sarvamHome':('SARVAM_HOME','.sarvam')}
SPECIAL={
 'astrbot':['Project("data/skills")','Home(".astrbot")'],
 'openclaw':['Home(".openclaw")','Home(".clawdbot")','Home(".moltbot")'],
 'codebuddy':['Project(".codebuddy")','Home(".codebuddy")'],
 'codex':['EnvHome { var: "CODEX_HOME", fallback: ".codex" }','Absolute("/etc/codex")'],
 'continue':['Project(".continue")','Home(".continue")'],
 'eve':['EveProject'],
 'jazz':['Home(".jazz")','Project(".jazz")'],
 'kilo':['Home(".kilo")','Home(".kilocode")'],
 'kimchi':['Home(".config/kimchi")'],
 'kimi-code-cli':['Home(".kimi-code")','Home(".kimi")'],
 'minimax-code':['Home(".minimax")','Absolute("/Applications/MiniMax Code.app")'],
 'posit-assistant':['Home(".posit/assistant")','Home(".positai")'],
 'replit':['Project(".replit")'],
 'zed':['Config("zed")','EnvJoin { var: "APPDATA", rel: "Zed" }','EnvJoin { var: "FLATPAK_XDG_CONFIG_HOME", rel: "zed" }'],
 'zcode':['Home(".zcode")','Absolute("/Applications/ZCode.app")'],
 'promptscript':['Project(".promptscript")','Project("promptscript.yaml")'],
 'universal':[],
}
def glob(g):
    if g=='undefined': return 'None'
    if g=='getOpenClawGlobalSkillsDir()': return 'Some(GlobalDir::OpenClaw)'
    m=re.fullmatch(r"join\((\w+), (.*)\)",g)
    base,rest=m.group(1),m.group(2)
    rel='/'.join(re.findall(r"'([^']*)'",rest))
    if base=='home': return f'Some(GlobalDir::Home("{rel}"))'
    if base=='configHome': return f'Some(GlobalDir::Config("{rel}"))'
    var,fb=ENV[base]; return f'Some(GlobalDir::EnvHome {{ var: "{var}", fallback: "{fb}", rel: "{rel}" }})'
def detect(key,blk):
    if key in SPECIAL: return SPECIAL[key]
    d=re.sub(r'\s+',' ',re.search(r"detectInstalled: async \(\) => \{\s*(.*?)\s*\},?$",blk,re.S).group(1))
    m=re.fullmatch(r"return existsSync\(join\((home|configHome), '([^']*)'\)\);",d)
    if m: return [('Home' if m.group(1)=='home' else 'Config')+f'("{m.group(2)}")']
    m=re.fullmatch(r"return existsSync\((\w+Home)\);",d)
    var,fb=ENV[m.group(1)]; return [f'EnvHome {{ var: "{var}", fallback: "{fb}" }}']
rows=[]
for m in re.finditer(r"\n  '?([a-z0-9-]+)'?: \{\n(.*?)\n  \},", body, re.S):
    key,blk=m.group(1),m.group(2)
    dn=re.search(r"displayName: '([^']*)'",blk).group(1)
    sd=re.search(r"skillsDir: '([^']*)'",blk).group(1)
    g=re.sub(r'\s+',' ',re.search(r"globalSkillsDir:\s*(.*?),\n",blk,re.S).group(1).strip())
    hide=('showInUniversalList: false' in blk)
    create=('createProjectSkillsDirByDefault: true' in blk)
    det=', '.join('Marker::'+x for x in detect(key,blk))
    rows.append(f'''    AgentDef {{
        id: "{key}",
        display_name: "{dn}",
        project_dir: "{sd}",
        global_dir: {glob(g)},
        detect: &[{det}],
        hidden_from_universal_list: {str(hide).lower()},
        create_project_dir_by_default: {str(create).lower()},
    }},''')
print(f'''//! Agent registry, generated from `src/agents.ts` in vercel-labs/skills
//! at commit `{commit}`. Regenerate with `tools/gen_agents.py` when upstream
//! adds or changes agents; do not hand-edit entries.

use super::{{AgentDef, GlobalDir, Marker}};

pub(super) static AGENTS: &[AgentDef] = &[
{chr(10).join(rows)}
];''')
