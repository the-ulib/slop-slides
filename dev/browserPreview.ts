// Dev-only Vite plugin: lets the UI run in a plain browser (no Tauri) against the real deck
// library, read-only. Used for visual checks; the desktop app uses the `slop://` protocol.
import { createHash } from "node:crypto";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import type { Plugin } from "vite";

const library = path.join(os.homedir(), "Documents", "SlopSlide");
const userTemplates = path.join(os.homedir(), ".slopslides", "templates");
const builtinTemplates = new URL("../src-tauri/templates/", import.meta.url);
const asset = (name: string) => fs.readFileSync(new URL(`../src-tauri/assets/${name}`, import.meta.url), "utf8");

/** Like src-tauri/src/templates.rs: the user's templates first, then the built-in ones they do not replace. */
function listTemplates() {
  const read = (id: string, file: string, builtin: boolean) => {
    const html = fs.readFileSync(file, "utf8");
    const slides = [...html.matchAll(/<section\b[^>]*\bclass=["'][^"']*\bslide\b[^"']*["'][^>]*>/gi)].flatMap(
      (m) => /\bid=["']([^"']+)["']/i.exec(m[0])?.[1] ?? [],
    );
    const title = /<title>([^<]*)<\/title>/i.exec(html)?.[1] ?? id;
    return { id, title, builtin, path: builtin ? null : path.dirname(file), slides };
  };
  const own = fs.existsSync(userTemplates)
    ? fs
        .readdirSync(userTemplates)
        .filter((id) => !id.startsWith(".") && fs.existsSync(path.join(userTemplates, id, "deck.html")))
        .map((id) => read(id, path.join(userTemplates, id, "deck.html"), false))
    : [];
  const builtin = fs
    .readdirSync(builtinTemplates)
    .filter((f) => f.endsWith(".html"))
    .map((f) => f.slice(0, -5))
    .filter((id) => !own.some((t) => t.id === id))
    .map((id) => read(id, new URL(`${id}.html`, builtinTemplates).pathname, true));
  return [...own, ...builtin];
}

/** A template's deck.html with the player runtime, as the app serves it. */
function templateHtml(id: string): string | null {
  const own = path.join(userTemplates, id, "deck.html");
  const builtin = new URL(`${id}.html`, builtinTemplates).pathname;
  const file = !id.includes("/") && fs.existsSync(own) ? own : !id.includes("/") && fs.existsSync(builtin) ? builtin : null;
  if (!file) return null;
  const html = fs.readFileSync(file, "utf8");
  if (html.includes("slopslide:runtime-js")) return html;
  const css = `<style>\n${asset("runtime.css")}</style><script>document.documentElement.setAttribute("data-slop-player", "");</script>`;
  const withCss = html.replace(/<head[^>]*>/i, (head) => head + css);
  const at = withCss.toLowerCase().lastIndexOf("</body");
  const js = `<script>\n${asset("runtime.js")}</script>`;
  return at < 0 ? withCss + js : withCss.slice(0, at) + js + withCss.slice(at);
}
const hash = (text: string) => createHash("sha1").update(text).digest("hex").slice(0, 12);

// Approximates src-tauri/src/html.rs: good enough for previews of well-formed decks.
function readDeck(id: string) {
  const dir = path.join(library, id);
  const file = path.join(dir, "deck.html");
  const html = fs.readFileSync(file, "utf8");
  const slideTags = [...html.matchAll(/<section\b[^>]*\bclass=["'][^"']*\bslide\b[^"']*["'][^>]*>/gi)];
  const slides = slideTags.map((match, index) => {
    const id = /\bid=["']([^"']+)["']/i.exec(match[0])?.[1] ?? `#${index + 1}`;
    const hidden = /\sdata-hidden\b/i.test(match[0]);
    const locked = /\sdata-locked\b/i.test(match[0]);
    const source = html.slice(match.index, html.indexOf("</section>", match.index));
    return { id, hash: hash(source), hidden, locked, moved: /<[^>]*\sdata-moved\b/i.test(source) };
  });
  const sections = [...html.matchAll(/<div\b[^>]*\bclass=["'][^"']*\bdeck-section\b[^"']*["'][^>]*>/gi)].map(
    (match, index) => ({
      index,
      title: /\bdata-title=["']([^"']*)["']/i.exec(match[0])?.[1] ?? "",
      before: slideTags.filter((slide) => slide.index < match.index).length,
    }),
  );
  const title = /<title>([^<]*)<\/title>/i.exec(html)?.[1] ?? id;
  return { id, title, path: dir, slides, sections, shellHash: hash(html), updatedMs: fs.statSync(file).mtimeMs };
}

export function browserPreview(): Plugin {
  return {
    name: "slopslide-browser-preview",
    apply: "serve",
    configureServer(server) {
      server.middlewares.use((req, res, next) => {
        const url = new URL(req.url ?? "/", "http://x");
        if (url.pathname === "/__api/decks") {
          const decks = fs.existsSync(library)
            ? fs
                .readdirSync(library)
                .filter((id) => fs.existsSync(path.join(library, id, "deck.html")))
                .map(readDeck)
            : [];
          res.setHeader("content-type", "application/json");
          return res.end(JSON.stringify(decks));
        }
        if (url.pathname === "/__api/templates") {
          res.setHeader("content-type", "application/json");
          return res.end(JSON.stringify(listTemplates()));
        }
        if (!url.pathname.startsWith("/__deck/")) return next();
        const template = /^\/__deck\/\.template\/([^/]+)\/deck\.html$/.exec(url.pathname);
        if (template) {
          const html = templateHtml(decodeURIComponent(template[1]!));
          if (html === null) {
            res.statusCode = 404;
            return res.end();
          }
          res.setHeader("content-type", "text/html; charset=utf-8");
          return res.end(html);
        }
        const file = path.resolve(library, decodeURIComponent(url.pathname.slice("/__deck/".length)));
        if (!file.startsWith(library + path.sep) || !fs.existsSync(file)) {
          res.statusCode = 404;
          return res.end();
        }
        if (file.endsWith(".html")) {
          res.setHeader("content-type", "text/html; charset=utf-8");
          // Like src-tauri/src/protocol.rs: `?pan` and `?show` add the pasteboard, `?edit` the
          // slide editor on top of it.
          const { searchParams: q } = url;
          const names = [
            ...(q.has("pan") || q.has("show") || q.has("edit") ? ["pasteboard.js"] : []),
            ...(q.has("edit") ? ["editor.js"] : []),
          ];
          if (names.length > 0) {
            const scripts = names
              .map((name) => `<script>\n${asset(name)}</script>\n`)
              .join("");
            const html = fs.readFileSync(file, "utf8");
            const at = html.toLowerCase().lastIndexOf("</body");
            return res.end(at < 0 ? html + scripts : html.slice(0, at) + scripts + html.slice(at));
          }
        }
        fs.createReadStream(file).pipe(res);
      });
    },
  };
}
