import { readFileSync, writeFileSync } from "node:fs";
import puppeteer from "puppeteer-core";
const uri = "data:image/png;base64," + readFileSync("brand/bean.png").toString("base64");
const b = await puppeteer.launch({ executablePath: "/usr/bin/google-chrome", args: ["--no-sandbox"] });
const p = await b.newPage(); await p.setViewport({width:500,height:500,deviceScaleFactor:2});
await p.setContent(`<body style="margin:0"><img id=i src="${uri}" width=500 height=500></body>`);
await p.evaluate(()=>document.getElementById("i").decode());
const facts = JSON.parse(readFileSync("/tmp/facts.json","utf8"));
const C = facts.CROP ?? {x:122,y:111,size:184};
const html = `
<style>body{margin:0;font:12px monospace}#w{position:relative;width:500px;height:500px}
#w img{width:500px;height:500px;display:block}
.c{position:absolute;border:2px dashed #ff2d55}
.e{position:absolute;border:2px solid #00e5ff}</style>
<div id=w><img src="${uri}">
<div class=c style="left:${C.x}px;top:${C.y}px;width:${C.size}px;height:${C.size}px"></div>
<div class=c style="left:71px;top:54px;width:${facts.head[2]-71}px;height:${facts.head[3]-54}px;border-color:#ffb703"></div>
<div class=e style="left:137px;top:56px;width:${facts.bubble[2]-137}px;height:${facts.bubble[3]-56}px"></div>
<div style="position:absolute;left:0;top:500px"></div></div>`;
await p.setContent(html);
await p.evaluate(()=>document.querySelector("img").decode());
await new Promise(r=>setTimeout(r,300));
const el = await p.$("#w");
await el.screenshot({ path: "/tmp/marked.png" });
console.log("crop =", JSON.stringify(C));
await b.close();
