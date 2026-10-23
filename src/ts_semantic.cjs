/* CodeGym's optional TypeScript symbol resolver.
 * Input: JSON on stdin, output: small JSON on stdout.
 * No writes, eval, package scripts, plugins, or network use.
 */
"use strict";
const fs = require("fs");
const path = require("path");

function typescript(repo) {
  const candidates = [
    path.join(repo, "node_modules", "typescript"),
    process.env.CODEGYM_TYPESCRIPT_PATH,
    process.env.APPDATA && path.join(process.env.APPDATA, "npm", "node_modules", "typescript"),
    "/usr/local/lib/node_modules/typescript",
    "/usr/lib/node_modules/typescript"
  ].filter(Boolean);
  // Requiring a known, installed compiler module doesn't execute any project TS.
  for (const loc of candidates) {
    try { return require(loc); } catch (e) {
      if (e.code !== "MODULE_NOT_FOUND" && e.code !== "ERR_PACKAGE_PATH_NOT_EXPORTED") throw e;
    }
  }
  try { return require("typescript"); } catch (_) {
    throw new Error("TypeScript compiler not found. Install with: npm install -g typescript, or set CODEGYM_TYPESCRIPT_PATH.");
  }
}

function main() {
  const data = JSON.parse(fs.readFileSync(0, "utf8"));
  // Rust's std::fs::canonicalize returns \\?\C:\... on Windows.
  // TypeScript's path utilities expect a conventional drive path.
  const repo = fs.realpathSync(String(data.repo).replace(/^\\\\\?\\/, ""));
  const definition = data.definition;
  const symbolName = data.symbol;
  if (!Array.isArray(data.files) || data.files.length > 4000 ||
      typeof definition !== "string" || typeof symbolName !== "string" ||
      !/^[a-zA-Z_$][\w$]{0,119}$/.test(symbolName) ||
      !data.files.includes(definition)) {
    throw new Error("Invalid project files or symbol.");
  }
  const within = (filename) => {
    const full = path.resolve(repo, filename);
    if (!full.startsWith(repo + path.sep)) throw new Error("Path outside repository");
    return full;
  };
  const fileNames = data.files.filter(p => (p.endsWith(".ts") || p.endsWith(".tsx")) && !p.split(/[\\/]/).includes("node_modules"))
    .map(within).filter(file => {
      if (!fs.existsSync(file) || fs.lstatSync(file).isSymbolicLink()) return false;
      return fs.realpathSync(file).startsWith(repo + path.sep);
    });
  const target = within(definition);
  const ts = typescript(repo);

  let current = path.dirname(target), cfg = null;
  while (current.startsWith(repo)) {
    const candidate = path.join(current, "tsconfig.json");
    if (fs.existsSync(candidate)) { cfg = candidate; break; }
    if (current === repo) break;
    const parent = path.dirname(current);
    if (parent === current) break;
    current = parent;
  }
  let compilerOptions = {
    allowJs: false, noEmit: true, skipLibCheck: true,
    moduleResolution: ts.ModuleResolutionKind.Node10,
    module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022,
  };
  if (cfg) {
    const loaded = ts.readConfigFile(cfg, ts.sys.readFile);
    if (!loaded.error) {
      const parsed = ts.parseJsonConfigFileContent(loaded.config, ts.sys, path.dirname(cfg));
      compilerOptions = { ...compilerOptions, ...parsed.options };
    }
  }
  // Never emit build artifacts even if a project's tsconfig enables it.
  compilerOptions.noEmit = true;
  compilerOptions.incremental = false;
  compilerOptions.tsBuildInfoFile = undefined;
  compilerOptions.plugins = undefined;
  // Build a lightweight reverse import graph without type-checking all files.
  // Follow only consumers of the requested definition, including re-export
  // barrels and tsconfig aliases. This avoids loading the whole monorepo AST.
  const normalize = file => {
    const name = path.normalize(file);
    return process.platform === "win32" ? name.toLowerCase() : name;
  };
  const tracked = new Set(fileNames.map(normalize));
  const reverse = new Map();
  const resolutionCache = ts.createModuleResolutionCache(
    repo, normalize, compilerOptions
  );
  for (const file of fileNames) {
    const sourceText = fs.readFileSync(file, "utf8");
    if (!sourceText.includes("import") && !sourceText.includes("export") &&
        !sourceText.includes("require")) continue;
    const imports = ts.preProcessFile(sourceText, true, true).importedFiles;
    for (const spec of imports) {
      const resolved = ts.resolveModuleName(
        spec.fileName, file, compilerOptions, ts.sys, resolutionCache
      ).resolvedModule;
      if (!resolved) continue;
      const dep = normalize(resolved.resolvedFileName);
      if (!tracked.has(dep)) continue;
      if (!reverse.has(dep)) reverse.set(dep, []);
      reverse.get(dep).push(file);
    }
  }
  const SCOPE_LIMIT = 96;
  const selected = new Map([[normalize(target), target]]);
  const queue = [target];
  let scopeTruncated = false;
  for (let index = 0; index < queue.length; index++) {
    for (const importer of (reverse.get(normalize(queue[index])) || [])) {
      const key = normalize(importer);
      if (selected.has(key)) continue;
      if (selected.size >= SCOPE_LIMIT) {
        scopeTruncated = true;
        continue;
      }
      selected.set(key, importer);
      queue.push(importer);
    }
  }

  // Even selected files can transitively import thousands of unrelated
  // modules. Restrict host resolution to the bounded graph above.
  const host = ts.createCompilerHost(compilerOptions);
  host.resolveModuleNames = (names, containingFile) => names.map(specifier => {
    const module = ts.resolveModuleName(
      specifier, containingFile, compilerOptions, ts.sys, resolutionCache
    ).resolvedModule;
    return module && selected.has(normalize(module.resolvedFileName))
      ? module : undefined;
  });
  const program = ts.createProgram([...selected.values()], compilerOptions, host);
  const checker = program.getTypeChecker();
  const sf = program.getSourceFile(target);
  if (!sf) throw new Error("Definition not in the TypeScript program.");

  let targetSymbol;
  const definitions = [];
  const isDeclaration = n =>
    ts.isFunctionDeclaration(n) || ts.isVariableDeclaration(n) ||
    ts.isMethodDeclaration(n) || ts.isGetAccessorDeclaration(n) ||
    ts.isSetAccessorDeclaration(n);
  function findDefinition(node) {
    if (isDeclaration(node) && node.name && ts.isIdentifier(node.name) &&
        node.name.text === symbolName) {
      const sym = checker.getSymbolAtLocation(node.name);
      if (sym && !targetSymbol) {
        targetSymbol = sym;
        definitions.push({
          path: definition, line: sf.getLineAndCharacterOfPosition(node.getStart(sf)).line + 1,
          kind: "definition", owner: null
        });
      }
    }
    ts.forEachChild(node, findDefinition);
  }
  findDefinition(sf);
  if (!targetSymbol) throw new Error("Declaration not found or unsupported symbol kind.");

  const resolve = sym =>
    sym && (sym.flags & ts.SymbolFlags.Alias) ? checker.getAliasedSymbol(sym) : sym;
  targetSymbol = resolve(targetSymbol);

  function owner(node) {
    let cursor = node.parent;
    while (cursor) {
      if ((ts.isFunctionDeclaration(cursor) || ts.isMethodDeclaration(cursor)) && cursor.name) {
        return cursor.name.getText();
      }
      if (ts.isArrowFunction(cursor) || ts.isFunctionExpression(cursor)) {
        const parent = cursor.parent;
        if (parent && ts.isVariableDeclaration(parent) && parent.name) {
          return parent.name.getText();
        }
      }
      cursor = cursor.parent;
    }
    return null;
  }

  const hits = definitions;
  let scanned = 0, truncated = scopeTruncated;
  for (const source of program.getSourceFiles()) {
    const full = path.resolve(source.fileName);
    if (!full.startsWith(repo + path.sep) || source.isDeclarationFile) continue;
    const rel = path.relative(repo, full).replace(/\\/g, "/");
    if (!data.files.includes(rel)) continue;
    scanned++;
    function visit(node) {
      if (hits.length >= 64) { truncated = true; return; }
      if (ts.isCallExpression(node)) {
        const callee = node.expression;
        const nameNode = ts.isPropertyAccessExpression(callee) ? callee.name : callee;
        const current = resolve(checker.getSymbolAtLocation(nameNode));
        if (current && current === targetSymbol) {
          hits.push({
            path: rel,
            line: source.getLineAndCharacterOfPosition(node.getStart(source)).line + 1,
            kind: "call", owner: owner(node)
          });
        }
      }
      ts.forEachChild(node, visit);
    }
    visit(source);
    if (truncated) break;
  }
  hits.sort((a,b) => (a.kind === "definition" ? -1 : 0) - (b.kind === "definition" ? -1 : 0)
    || a.path.localeCompare(b.path) || a.line-b.line);
  process.stdout.write(JSON.stringify({hits, scanned, truncated, scoped:true}));
}
try { main(); }
catch (error) {
  process.stderr.write(String(error.message || error).slice(0, 700));
  process.exitCode = 2;
}
