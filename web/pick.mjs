import { readFileSync, writeFileSync } from "node:fs";
import puppeteer from "puppeteer-core";
const uri = "data:image/png;base64," + readFileSync("brand/bean.png").toString("base64");
const b = await puppeteer.launch({ executablePath: "/usr/bin/google-chrome", args: ["--no-sandbox"] });
const p = await b.newPage(); await p.goto("about:blank");
const out = await p.evaluate(async (uri) => {
  const img = new Image(); img.src = uri; await img.decode();
  const W = img.width, H = img.height;
  const c = document.createElement("canvas"); c.width = W; c.height = H;
  const g = c.getContext("2d"); g.drawImage(img, 0, 0);
  const d = g.getImageData(0, 0, W, H).data;
  const A = (x,y) => d[(y*W+x)*4+3];
  const L = (x,y) => { const i=(y*W+x)*4; return 0.299*d[i]+0.587*d[i+1]+0.114*d[i+2]; };
  const cx = W/2, cy = H/2;
  // bán kính trong của huy hiệu ~227 -> dùng 214 để loại viền đen
  const R = 214;
  const inBadge = (x,y) => A(x,y) > 200 && Math.hypot(x-cx, y-cy) <= R;
  const dog = (x,y) => inBadge(x,y) && L(x,y) < 80;
  const bub = (x,y) => inBadge(x,y) && L(x,y) > 228;
  const bbox = (pred) => { let a=1e9,bb=-1,cc=1e9,dd=-1;
    for(let y=0;y<H;y++)for(let x=0;x<W;x++){ if(!pred(x,y))continue;
      if(x<a)a=x; if(x>bb)bb=x; if(y<cc)cc=y; if(y>dd)dd=y; }
    return a<1e9?[a,cc,bb,dd]:null; };
  const head = bbox(dog), bubble = bbox(bub);
  // đôi mắt: cụm sáng nhỏ nằm trong vùng chó
  const eyes = [];
  for (let y=100;y<300;y+=2) for (let x=100;x<340;x+=2) {
    if (!dog(x,y)) continue;
    if (L(x,y) > 150) { const k=eyes.find(e=>Math.hypot(e.x-x,e.y-y)<28); if(k){k.n++;} else eyes.push({x,y,n:1}); }
  }
  eyes.sort((a,b2)=>b2.n-a.n);
  return { W,H,head,bubble,eyes: eyes.slice(0,4) };
}, uri);
console.log(JSON.stringify(out));
writeFileSync("/tmp/facts.json", JSON.stringify(out));
await b.close();
