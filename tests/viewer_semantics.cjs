// Run with: node --test tests/viewer_semantics.cjs (no browser or npm dependencies).
const { test } = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const vm = require('node:vm');
const path = require('node:path');
const assets = path.join(__dirname, '../src/assets');
function contrast(a, b) {
  const luminance = hex => {
    const rgb = hex.match(/[a-f\d]{2}/gi).map(v => parseInt(v, 16) / 255)
      .map(v => v <= 0.04045 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4);
    return rgb[0] * 0.2126 + rgb[1] * 0.7152 + rgb[2] * 0.0722;
  };
  const x = luminance(a), y = luminance(b);
  return (Math.max(x, y) + 0.05) / (Math.min(x, y) + 0.05);
}

function viewer(dark = false) {
  const css = fs.readFileSync(path.join(assets, 'graph.css'), 'utf8');
  const variables = text => Object.fromEntries([...text.matchAll(/(--[\w-]+):\s*([^;]+);/g)].map(m => [m[1], m[2]]));
  let colors = variables(css.split('@media')[0]);
  if (dark) colors = { ...colors, ...variables(css.split('@media')[1].split('* {')[0]) };
  const legend = { innerHTML: '' };
  const strokes = [];
  const ctx = new Proxy({
    stroke() { strokes.push({ color: this.strokeStyle, dash: this.dash, alpha: this.globalAlpha }); },
    setLineDash(dash) { this.dash = dash; },
  }, { get: (target, key) => key in target ? target[key] : () => {} });
  const context = vm.createContext({
    console, ctx, setTimeout, clearTimeout, Math,
    window: { __GRAPH_DATA__: { nodes: [], edges: [] }, matchMedia: () => ({ matches: dark, addEventListener() {} }) },
    document: { documentElement: {}, getElementById: id => id === 'legend' ? legend : null },
    getComputedStyle: () => ({ getPropertyValue: key => colors[key] || '' }),
    requestAnimationFrame() {},
  });
  const run = source => vm.runInContext(source, context);
  run(`const state = { colorMode:'kind', kinds:new Set(['calls','inherits']), hiddenFamilies:new Set(), showAmbiguous:true };
    let nodes=[], links=[], inFileRegime=true, selected=null, hovered=null;
    const expanded=new Set(); const transform={x:0,y:0,k:1};
    const dpr=1, width=800, height=600;
    function highlightSet(){return null;} function activeNodeId(){return selected;}
    function hit(){return false;} function esc(s){return String(s).replace(/[&<>\"]/g,c=>({'&':'&amp;','<':'&lt;','>':'&gt;','\"':'&quot;'}[c]));}`);
  for (const file of ['00-data.js', '20-theme.js', '30-model.js', '50-render.js', '85-legend.js']) {
    run(fs.readFileSync(path.join(assets, 'graph', file), 'utf8'));
  }
  return { run, legend, strokes };
}

for (const dark of [false, true]) test(`families, colors and exact-kind glyphs in ${dark ? 'dark' : 'light'} theme`, () => {
  const { run } = viewer(dark);
  assert.equal(run("nodeFamily('constructor')"), 'callable');
  for (const kind of ['field', 'property', 'constant', 'enum_member']) assert.equal(run(`nodeFamily('${kind}')`), 'data');
  assert.equal(run("nodeFamily('component')"), 'component');
  assert.equal(run("nodeFamily('impl')"), 'organization');
  assert.equal(run("nodeFamily('unknown')"), 'other');
  assert.equal(run("new Set(['class','function','field','module','component','macro'].map(kindColor)).size"), 6);
  assert.equal(run('new Set(Object.keys(EDGE_STYLES).map(edgeColor)).size'), 7);
  assert.notEqual(run("nodeShape('class')"), run("nodeShape('method')"));
  assert.notEqual(run('KIND_GLYPHS.constructor'), run('KIND_GLYPHS.method'));
  assert.equal(run("kindColor('unknown')"), run('COL.ext'));
  for (const color of JSON.parse(run('JSON.stringify([...Object.values(NODE_FAMILIES).map(f => COL[f.color]), ...Object.keys(EDGE_STYLES).map(edgeColor)])'))) {
    assert.ok(contrast(color, run('COL.bg')) >= 3, `${color} must contrast with canvas`);
  }
  assert.equal(run("state.colorMode='community'; nodeShape('field')"), 'diamond');
});

test('selection preserves relation colors; ambiguity uses dashes independent of confidence', () => {
  const { run, strokes } = viewer();
  run(`nodes=[{id:'a',label:'A',kind:'class',x:40,y:40},{id:'b',label:'run',kind:'method',x:140,y:40}];
    links=[{source:nodes[0],target:nodes[1],kind:'calls',count:1,ambiguous:true}]; selected='a'; draw();`);
  assert.equal(strokes[0].color, run("edgeColor('calls')"));
  assert.equal(Array.from(strokes[0].dash).join(','), '5,4');
  run('links[0].ambiguous=false; draw();');
  assert.equal(Array.from(strokes[3].dash).length, 0);
});

test('live legend includes data symbols, component and relations, escapes labels, and follows filters', () => {
  const { run, legend } = viewer();
  run(`nodes=[{kind:'field'},{kind:'component'},{kind:'constructor'}]; links=[{kind:'implements',ambiguous:true}]; buildLegend();`);
  assert.match(legend.innerHTML, /Fields \/ properties \/ data/);
  assert.match(legend.innerHTML, /<code>f<\/code><span>field/);
  assert.match(legend.innerHTML, /<code>new<\/code><span>constructor/);
  assert.match(legend.innerHTML, /Components/);
  assert.match(legend.innerHTML, /implements/);
  assert.match(legend.innerHTML, /Ambiguous guess included/);
  run('nodes=[]; links=[]; buildLegend();');
  assert.doesNotMatch(legend.innerHTML, /Ambiguous guess included|Components|implements/);
  run(`COMMUNITIES.push({id:1,label:'<script>',size:2}); state.colorMode='community'; nodes=[{kind:'class',community:1}]; buildLegend();`);
  assert.match(legend.innerHTML, /&lt;script&gt;/);
  assert.doesNotMatch(legend.innerHTML, /<script>/);
  assert.match(legend.innerHTML, /Subsystem colors/);
  assert.match(legend.innerHTML, /Symbol shapes and glyphs/);
  assert.match(legend.innerHTML, /legend-count">1<\/span>/);
  assert.doesNotMatch(legend.innerHTML, /legend-count">2<\/span>/);
});

test('legend explains each relation direction, certainty and selection without equating solid with precise', () => {
  const { run, legend } = viewer();
  run("nodes=[{kind:'class'},{kind:'method'}]; links=Object.keys(EDGE_STYLES).map(kind=>({kind})); buildLegend();");
  for (const text of ['caller → callable', 'derived type → base type', 'implementing type → interface / trait', 'overriding method → base method', 'owner — member (no arrow)', 'A solid line is not proof of precise resolution', 'Selection preserves relation colors']) {
    assert.ok(legend.innerHTML.includes(text), text);
  }
  assert.match(legend.innerHTML, /data-family="type"/);
  assert.match(legend.innerHTML, /Alternatives are not extra edges/);
});

test('filter guesses before aggregation so resolved links survive mixed groups', () => {
  const { run } = viewer();
  run(`nodeById.set('file:a',{id:'file:a',kind:'file'}); nodeById.set('file:b',{id:'file:b',kind:'file'});
    expanded.add('file:a'); crossEdges.push(
      {source:'file:a',target:'file:b',srcFile:'file:a',tgtFile:'file:b',kind:'calls',resolution:{ambiguous:true}},
      {source:'file:a',target:'file:b',srcFile:'file:a',tgtFile:'file:b',kind:'calls',resolution:{ambiguous:false}});`);
  assert.equal(run('buildLinks()[0].count'), 2);
  assert.equal(run('buildLinks()[0].ambiguous'), true);
  assert.equal(run('state.showAmbiguous=false; buildLinks()[0].count'), 1);
  assert.equal(run('buildLinks()[0].ambiguous'), false);
  run(`DATA.nodes.push({id:'sym:field',kind:'field'}); parentOf.set('sym:field','file:a');`);
  assert.equal(run('visibleNodeList().length'), 1);
  assert.equal(run("state.hiddenFamilies.add('data'); visibleNodeList().length"), 0);
});

test('all viewer fragments form a syntactically valid shipped script', () => {
  const files = fs.readdirSync(path.join(assets, 'graph')).filter(f => f.endsWith('.js')).sort();
  new vm.Script('(function(){"use strict";\n' + files.map(f => fs.readFileSync(path.join(assets, 'graph', f), 'utf8')).join('\n') + '\n})();');
});

test('every canonical symbol kind has an explicit family and a glyph except unknown', () => {
  const { run } = viewer();
  const source = fs.readFileSync(path.join(__dirname, '../src/analysis/semantic.rs'), 'utf8');
  const kinds = [...source.matchAll(/Self::\w+ => "(\w+)"/g)].map(m => m[1]);
  assert.ok(kinds.length >= 25);
  for (const kind of kinds) {
    if (kind === 'unknown') continue;
    assert.notEqual(run(`nodeFamily('${kind}')`), 'other', kind);
    assert.equal(run(`typeof KIND_GLYPHS['${kind}']`), 'string', kind);
  }
});

test('selective coverage is shown without claiming full verification', () => {
  const {run,legend}=viewer();
  run(`DATA.precision={requested_mode:'selective',complete:false,languages:[{verified:2,unresolved:1,skipped:5,pending:3}]};buildLegend();`);
  assert.match(legend.innerHTML,/2 verified, 1 unresolved, 5 skipped, 3 pending/);
  assert.match(legend.innerHTML,/Selective completion does not verify every reference/);
});
