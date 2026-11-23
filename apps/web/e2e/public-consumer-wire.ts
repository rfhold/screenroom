import type {Page} from '@playwright/test';
import {userId} from './maintenance-server';
export const source='20000000-0000-4000-8000-000000000001',library='20000000-0000-4000-8000-000000000002';
const variable=(n:number)=>{const bytes=[];do{bytes.push((n&127)|(n>127?128:0));n>>>=7;}while(n);return Buffer.from(bytes);};
export const text=(n:number,value:string|Buffer)=>{const b=typeof value==='string'?Buffer.from(value):value;return Buffer.concat([variable(n*8+2),variable(b.length),b]);};
export const bit=(n:number,value=1)=>Buffer.concat([variable(n*8),variable(value)]);
export const msg=(...parts:Buffer[])=>Buffer.concat(parts);
export function decode(data:Buffer){let i=0;const read=()=>{let v=0,s=0,b;do{b=data[i++];v|=(b&127)<<s;s+=7;}while(b&128);return v;};const result=new Map<number,Buffer>();while(i<data.length){const tag=read(),wire=tag&7;if(wire===2){const length=read();result.set(tag>>>3,data.subarray(i,i+length));i+=length;}else if(wire===0){const start=i;read();result.set(tag>>>3,data.subarray(start,i));}else throw Error(`Unexpected wire ${wire}`);}return result;}
export type Call={method:string;fields:Map<number,Buffer>};
export const value=(c:Call,n:number)=>c.fields.get(n)?.toString()??'';
export const nested=(c:Call,n:number)=>decode(c.fields.get(n)??Buffer.alloc(0));
export const token=(c:Call,n:number)=>nested(c,n).get(2)?.toString()??'';
export const item=(id:string,kind='movie',parent='')=>msg(text(1,id),text(2,id),text(3,kind[0].toUpperCase()+kind.slice(1)),...(parent?[text(4,parent)]:[]),text(7,`Metadata ${id}`));
function frame(data:Buffer,status=0){const h=Buffer.alloc(5);h.writeUInt32BE(data.length,1);const trailer=Buffer.from(`grpc-status: ${status}\r\ngrpc-message: unavailable\r\n`),t=Buffer.alloc(5);t[0]=128;t.writeUInt32BE(trailer.length,1);return Buffer.concat([h,data,t,trailer]);}
export async function consumerWire(page:Page,origin:string,handler:(call:Call)=>Promise<Buffer|false|undefined>|Buffer|false|undefined,semantic=true){
 const calls:Call[]=[];
 await page.route('**/*',async route=>{
  const url=new URL(route.request().url());if(url.origin!==origin)return route.abort();if(!/^\/screenroom\.(catalog|user)\.v1\./.test(url.pathname))return route.continue();
  const body=route.request().postDataBuffer()!;const call={method:url.pathname.split('/').at(-1)!,fields:decode(body.subarray(5,5+body.readUInt32BE(1)))};calls.push(call);
  let data=await handler(call);
  if(data===undefined){
   data=Buffer.alloc(0);
   if(call.method==='GetCatalogInfo')data=text(1,Buffer.alloc(0));
   if(call.method==='ListLibraries')data=text(1,msg(text(1,library),text(2,'Protocol library')));
   if(call.method==='GetCurrentUser')data=text(1,msg(text(1,userId),text(2,'Protocol user')));
   if(call.method==='GetSearchCapabilities')data=msg(text(1,'text'),text(1,'fuzzy'),...(semantic?[text(1,'semantic')]:[]),text(2,'Movie'),text(2,'Series'),text(2,'Episode'),text(3,'all'),text(3,'jellyfin'),bit(5,160));
  }
  await route.fulfill({status:200,headers:{'content-type':'application/grpc-web+proto','access-control-allow-origin':'*'},body:frame(data===false?Buffer.alloc(0):data,data===false?14:0)}).catch(()=>{});
 });return calls;
}
