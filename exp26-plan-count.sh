#!/usr/bin/env bash
#
# exp26-plan-count.sh — the UI-revamp baseline measurement.
#
# ONE script, run before and after the revamp, on the same tree layout, so the
# 80 % claim is a subtraction rather than an estimate. It reads source only and
# writes nothing.
#
#   ./exp26-plan-count.sh [repo-root]        # human table
#   ./exp26-plan-count.sh --json [repo-root] # machine-readable, for diffing
#
# Metrics, per package, over SHIPPING files only (`*.test.*`, `*.stories.*` and
# `src/testing/**` are counted separately and reported on their own row):
#
#   a  component_files    .tsx files that export at least one Capitalised symbol
#      component_exports  those exported Capitalised symbols, counted
#      styling_files      source files carrying at least one class string —
#                         "files that make a styling decision". THIS is the
#                         number the "80 % fewer UI components" target is read
#                         against, outside @vpay/ui.
#   b  classname_sites    occurrences of `className` in the source
#   c  class_tokens       DISTINCT whitespace-separated tokens across every
#                         class string (definition below)
#      class_tokens_total the same, with duplicates, so churn is visible too
#   d  css_lines          non-blank, non-comment lines in every *.css in scope
#      apply_sites        occurrences of `@apply`
#   e  inline_styles      occurrences of `style={{`
#
# Two aggregate rows are printed under the table and are what the gate reads:
#   TOTAL (shipping)          every scope, @vpay/ui included
#   OUTSIDE @vpay/ui          every scope but the shared library. `class_tokens
#                             _distinct` there is a UNION across packages, not
#                             a sum, so a token used by two apps counts once.
#
# KNOWN BLIND SPOT, stated so the numbers are not oversold: a class name held in
# a plain lookup object that is not an argument to cva/cn/clsx/twMerge is not
# seen. At the 2026-09-07 baseline that is exactly three tokens — `alert-success`
# `alert-error` `alert-info` in `TONE_CLASS` in
# frontends/apps/checkout/src/components/screens.tsx. The revamp moves that map
# into a `cva` variant, where this script DOES see it, so the blind spot makes
# the "after" number bigger rather than smaller. The measurement is therefore
# conservative against the reduction being claimed.
#
# A "class string" is deliberately narrow and mechanical, so two people get the
# same number:
#   - the value of a `className=` attribute — "…", '…', {'…'}, {"…"}, or a
#     template literal with every `${…}` deleted; and
#   - every string literal lexically inside a `cva(`, `cn(`, `clsx(` or
#     `twMerge(` call (balanced parens).
# Nothing else. A class name assembled at runtime out of a lookup table that is
# not inside one of those calls is NOT counted — `--list` prints every token so
# a reader can see exactly what was and was not picked up.
#
set -euo pipefail

json=0
list=0
root=""
for arg in "$@"; do
  case "$arg" in
    --json) json=1 ;;
    --list) list=1 ;;
    -*) echo "unknown flag: $arg" >&2; exit 2 ;;
    *) root="$arg" ;;
  esac
done

if [ -z "$root" ]; then
  root="$(git rev-parse --show-toplevel 2>/dev/null || pwd)"
fi

# Assert we are pointed at this repository and not at a sibling worktree by
# accident — several agents share a scratchpad and a mistyped path here would
# silently produce a baseline for the wrong tree.
for marker in justfile AGENTS.md frontends/packages/ui/package.json; do
  if [ ! -e "$root/$marker" ]; then
    echo "exp26-plan-count.sh: $root does not look like the vpay repo root (missing $marker)" >&2
    exit 2
  fi
done

export EXP26_ROOT="$root" EXP26_JSON="$json" EXP26_LIST="$list"

node --input-type=module <<'NODE'
import { readFileSync, readdirSync, statSync } from 'node:fs';
import { join, relative } from 'node:path';

const ROOT = process.env.EXP26_ROOT;
const AS_JSON = process.env.EXP26_JSON === '1';
const LIST = process.env.EXP26_LIST === '1';

/** The five scopes the revamp touches. Add one here, not in three places. */
const SCOPES = [
  ['@vpay/ui', 'frontends/packages/ui'],
  ['@vpay/tokens', 'frontends/packages/tokens'],
  ['@vpay/config', 'frontends/packages/config'],
  ['@vpay/checkout', 'frontends/apps/checkout'],
  ['@vpay/dashboard', 'frontends/apps/dashboard'],
  ['@vpay-examples/shop', 'examples/shop'],
];

const SKIP_DIR = new Set(['node_modules', '.next', 'dist', 'storybook-static', '.turbo', 'zenstack', '.zenstack']);

function walk(dir, out = []) {
  let entries;
  try { entries = readdirSync(dir); } catch { return out; }
  for (const name of entries) {
    if (SKIP_DIR.has(name)) continue;
    const full = join(dir, name);
    let st;
    try { st = statSync(full); } catch { continue; }
    if (st.isDirectory()) walk(full, out);
    else out.push(full);
  }
  return out;
}

const isSource = (f) => /\.(tsx|ts|jsx|js)$/.test(f) && !/\.d\.ts$/.test(f);
const isCss = (f) => /\.css$/.test(f);
const isNonShipping = (f) =>
  /\.test\.[jt]sx?$/.test(f) ||
  /\.stories\.[jt]sx?$/.test(f) ||
  /(^|\/)(src\/)?testing\//.test(f) ||
  /(^|\/)vitest\.setup\./.test(f);

/** Delete `${ … }` from a template literal body, leaving only its literal text. */
function stripInterpolations(s) {
  let out = '';
  for (let i = 0; i < s.length; i++) {
    if (s[i] === '$' && s[i + 1] === '{') {
      let depth = 1;
      i += 2;
      while (i < s.length && depth > 0) {
        if (s[i] === '{') depth++;
        else if (s[i] === '}') depth--;
        i++;
      }
      i--;
      out += ' ';
    } else out += s[i];
  }
  return out;
}

/** The slice from `open` (index of the `(`) to its matching `)`. */
function balanced(src, open) {
  let depth = 0;
  for (let i = open; i < src.length; i++) {
    if (src[i] === '(') depth++;
    else if (src[i] === ')') { depth--; if (depth === 0) return src.slice(open + 1, i); }
  }
  return src.slice(open + 1);
}

const STRING_LITERAL = /'([^'\\\n]*(?:\\.[^'\\\n]*)*)'|"([^"\\\n]*(?:\\.[^"\\\n]*)*)"|`([^`\\]*(?:\\.[^`\\]*)*)`/g;

function classStrings(src) {
  const found = [];

  // 1. className= attributes.
  const attr = /className\s*=\s*(\{?)/g;
  let m;
  while ((m = attr.exec(src)) !== null) {
    const rest = src.slice(m.index + m[0].length);
    const q = rest[0];
    if (q === '"' || q === "'" || q === '`') {
      const close = q === '`' ? /`/ : new RegExp(q);
      const end = rest.slice(1).search(close);
      if (end >= 0) {
        const body = rest.slice(1, 1 + end);
        found.push(q === '`' ? stripInterpolations(body) : body);
      }
    }
  }

  // 2. every string literal inside cva( / cn( / clsx( / twMerge(.
  const call = /\b(cva|cn|clsx|twMerge)\s*\(/g;
  while ((m = call.exec(src)) !== null) {
    const body = balanced(src, m.index + m[0].length - 1);
    let s;
    const re = new RegExp(STRING_LITERAL.source, 'g');
    while ((s = re.exec(body)) !== null) {
      const lit = s[1] ?? s[2] ?? s[3] ?? '';
      found.push(s[3] !== undefined ? stripInterpolations(lit) : lit);
    }
  }

  return found;
}

/** Exported symbols whose name starts with a capital — the component surface. */
function componentExports(src) {
  const names = new Set();
  const re = /export\s+(?:default\s+)?(?:async\s+)?(?:function|const|class)\s+([A-Z][A-Za-z0-9_]*)/g;
  let m;
  while ((m = re.exec(src)) !== null) names.add(m[1]);
  const reExport = /export\s*\{([^}]*)\}/g;
  while ((m = reExport.exec(src)) !== null) {
    for (const part of m[1].split(',')) {
      const name = part.trim().split(/\s+as\s+/).pop()?.trim() ?? '';
      // `type Foo` re-exports are types, not components.
      if (/^[A-Z][A-Za-z0-9_]*$/.test(name) && !/^\s*type\s/.test(part)) names.add(name);
    }
  }
  return names;
}

const countOf = (src, re) => (src.match(re) ?? []).length;

/** Non-blank, non-comment CSS lines. Block comments are stripped first. */
function cssLines(src) {
  return src
    .replace(/\/\*[\s\S]*?\*\//g, '')
    .split('\n')
    .filter((l) => l.trim() !== '')
    .length;
}

const rows = [];
const globalTokens = new Set();
const outsideTokens = new Set();

for (const [name, rel] of SCOPES) {
  const dir = join(ROOT, rel);
  const files = walk(dir);
  const bucket = (nonShipping) => ({
    files: 0, component_files: 0, component_exports: 0, styling_files: 0,
    classname_sites: 0, class_tokens_total: 0, tokens: new Set(),
    inline_styles: 0, apply_sites: 0, css_files: 0, css_lines: 0,
    nonShipping,
  });
  const ship = bucket(false);
  const test = bucket(true);

  for (const f of files) {
    const relPath = relative(ROOT, f);
    const target = isNonShipping(relative(dir, f)) ? test : ship;
    if (isCss(f)) {
      const src = readFileSync(f, 'utf8');
      target.css_files++;
      target.css_lines += cssLines(src);
      target.apply_sites += countOf(src, /@apply\b/g);
      continue;
    }
    if (!isSource(f)) continue;
    const src = readFileSync(f, 'utf8');
    target.files++;
    target.classname_sites += countOf(src, /className/g);
    target.inline_styles += countOf(src, /style=\{\{/g);
    target.apply_sites += countOf(src, /@apply\b/g);
    if (/\.tsx$/.test(f)) {
      const exps = componentExports(src);
      if (exps.size > 0) { target.component_files++; target.component_exports += exps.size; }
    }
    let sawToken = false;
    for (const s of classStrings(src)) {
      for (const tok of s.split(/\s+/)) {
        if (tok === '') continue;
        sawToken = true;
        target.class_tokens_total++;
        target.tokens.add(tok);
        if (!target.nonShipping) {
          globalTokens.add(tok);
          if (name !== '@vpay/ui') outsideTokens.add(tok);
        }
      }
    }
    if (sawToken) target.styling_files++;
    if (LIST && !target.nonShipping) {
      const toks = classStrings(src).flatMap((s) => s.split(/\s+/)).filter(Boolean);
      if (toks.length) console.error(`  ${relPath}: ${[...new Set(toks)].sort().join(' ')}`);
    }
  }
  rows.push({ name, rel, ship, test });
}

const num = (b) => ({
  files: b.files,
  component_files: b.component_files,
  component_exports: b.component_exports,
  styling_files: b.styling_files,
  classname_sites: b.classname_sites,
  class_tokens_distinct: b.tokens.size,
  class_tokens_total: b.class_tokens_total,
  css_files: b.css_files,
  css_lines: b.css_lines,
  apply_sites: b.apply_sites,
  inline_styles: b.inline_styles,
});

const totals = (key, rowset = rows) => rowset.reduce((acc, r) => {
  const n = num(r[key]);
  for (const k of Object.keys(n)) acc[k] = (acc[k] ?? 0) + n[k];
  return acc;
}, {});

const outsideRows = rows.filter((r) => r.name !== '@vpay/ui');

if (AS_JSON) {
  console.log(JSON.stringify({
    root: ROOT,
    packages: rows.map((r) => ({ name: r.name, path: r.rel, shipping: num(r.ship), tests_and_stories: num(r.test) })),
    totals: { shipping: totals('ship'), tests_and_stories: totals('test') },
    outside_vpay_ui: {
      ...totals('ship', outsideRows),
      class_tokens_distinct: outsideTokens.size,
    },
    distinct_class_tokens_repo_wide_shipping: globalTokens.size,
  }, null, 2));
} else {
  const cols = ['files', 'component_files', 'component_exports', 'styling_files', 'classname_sites', 'class_tokens_distinct', 'class_tokens_total', 'css_files', 'css_lines', 'apply_sites', 'inline_styles'];
  const head = ['package', ...cols];
  const body = rows.map((r) => [r.name, ...cols.map((c) => String(num(r.ship)[c]))]);
  const t = totals('ship');
  body.push(['TOTAL (shipping)', ...cols.map((c) => String(t[c]))]);
  const o = { ...totals('ship', outsideRows), class_tokens_distinct: outsideTokens.size };
  body.push(['OUTSIDE @vpay/ui', ...cols.map((c) => String(o[c]))]);
  const tt = totals('test');
  body.push(['tests+stories', ...cols.map((c) => String(tt[c]))]);
  const widths = head.map((h, i) => Math.max(h.length, ...body.map((r) => r[i].length)));
  const line = (r) => r.map((c, i) => c.padEnd(widths[i])).join('  ');
  console.log(line(head));
  console.log(widths.map((w) => '-'.repeat(w)).join('  '));
  for (const r of body) console.log(line(r));
  console.log('');
  console.log(`distinct class tokens, all shipping source:  ${globalTokens.size}`);
  console.log(`distinct class tokens, OUTSIDE @vpay/ui:      ${outsideTokens.size}   <-- the 80 % gate reads this`);
}
NODE
