import {expect,test,type Page} from '@playwright/test';
import {maintenanceServer,userId} from './maintenance-server';
import {consumerWire,source,library,text,bit,msg,item,value,nested,token,decode,type Call} from './public-consumer-wire';
// Minimal generated wire messages, not a bundled catalog or provider dataset.
const varint=(n:number)=>{const a=[];do{a.push((n&127)|(n>127?128:0));n>>>=7;}while(n);return Buffer.from(a);};
const field=(n:number,value:string|Buffer)=>{const b=typeof value==='string'?Buffer.from(value):value;return Buffer.concat([varint(n*8+2),varint(b.length),b]);};
const message=(...fields:Buffer[])=>Buffer.concat(fields);
function frame(data:Buffer){const h=Buffer.alloc(5);h.writeUInt32BE(data.length,1);const trailer=Buffer.from('grpc-status: 0\r\n'),end=Buffer.alloc(5);end[0]=128;end.writeUInt32BE(trailer.length,1);return Buffer.concat([h,data,end,trailer]);}
async function publicWire(page:Page,origin:string,options:{libraries?:boolean;delayHome?:boolean;pages?:boolean;fail?:boolean}={}){
 let release=()=>{},reads=0;const calls:string[]=[];
 await page.route('**/*',async route=>{
  const url=new URL(route.request().url());if(url.origin===origin)return route.continue();
  if(!url.pathname.startsWith('/screenroom.'))return route.abort();
  const method=url.pathname.split('/').at(-1)!;calls.push(method);let data=Buffer.alloc(0);
  if(method==='ListLibraries'&&options.fail)return route.abort();
  if(method==='ListLibraries'&&options.libraries)data=field(1,message(field(1,'library'),field(2,'Test library')));
  if(method==='GetCatalogInfo')data=field(1,Buffer.alloc(0));
  if(method==='GetSearchCapabilities')data=message(field(1,'text'),field(1,'fuzzy'),field(2,'movie'),field(2,'series'),field(3,'all'),field(3,'jellyfin'));
  if(method==='GetCurrentUser')data=field(1,message(field(1,userId),field(2,'Configured user')));
  if(method==='GetHome'&&options.delayHome)await new Promise<void>(r=>release=r);
  if(method==='ListLibraryItems'){
   reads++;if(options.pages){const start=reads===1?0:100;data=message(...Array.from({length:100},(_,i)=>field(1,message(field(1,`test-item-${start+i}`),field(2,`Test item ${start+i}`),field(3,'movie')))),...(reads===1?[field(2,'next-test-page')]:[]));}
  }
  await route.fulfill({status:200,headers:{'content-type':'application/grpc-web+proto','access-control-allow-origin':'*','grpc-status':'0'},body:frame(data)}).catch(()=>{});
 });return {calls,release:()=>release(),reads:()=>reads};
}
for(const enabled of [true,false])test(`healthy empty Home ${enabled?'offers setup':'reports unavailable attachment'} without demo bootstrap`,async({page})=>{const s=await maintenanceServer(enabled,{empty:true});try{await publicWire(page,s.origin);await page.goto(s.origin);await expect(page.getByRole('heading',{name:'Your library starts here',exact:true})).toBeVisible();if(enabled){await page.getByRole('link',{name:'Add server',exact:true}).click();await expect(page.getByRole('heading',{name:'Your servers',exact:true})).toBeVisible();}else{await expect(page.getByText('No libraries are available. Server setup is unavailable on this web attachment.',{exact:true})).toBeVisible();expect(s.calls).toHaveLength(0);}}finally{await s.close();}});
test('late public Home read cannot replace independent Maintenance after navigation',async({page})=>{const s=await maintenanceServer(true,{empty:true});try{const wire=await publicWire(page,s.origin,{delayHome:true});await page.goto(s.origin);await expect.poll(()=>wire.calls.includes('GetHome')).toBe(true);await page.locator('body').evaluate(()=>{const a=document.createElement('a');a.href='/?screen=maintenance';a.textContent='Open maintenance';document.body.append(a);});await page.getByRole('link',{name:'Open maintenance',exact:true}).click();await expect(page.getByRole('heading',{name:'Maintenance',exact:true})).toBeVisible();wire.release();await expect(page.getByRole('heading',{name:'Your servers',exact:true})).toBeVisible();await expect(page.getByRole('heading',{name:'Home',exact:true})).toHaveCount(0);}finally{await s.close();}});
test('neutral generated pages append through actual public WASM without fixture content',async({page})=>{const s=await maintenanceServer();try{const wire=await publicWire(page,s.origin,{libraries:true,pages:true});await page.goto(`${s.origin}/?screen=browse&library=all`);await expect(page.locator('[aria-label="Catalog"] > *')).toHaveCount(100);await page.getByRole('button',{name:'Load more',exact:true}).scrollIntoViewIfNeeded();await expect(page.locator('[aria-label="Catalog"] > *')).toHaveCount(200);expect(wire.reads()).toBe(2);await page.getByRole('link',{name:'Maintenance',exact:true}).click();await expect(page.getByRole('heading',{name:'Maintenance',exact:true})).toBeVisible();}finally{await s.close();}});
const scoped=(screen:string,extra='')=>`/?screen=${screen}&library=${library}&source_id=${source}${extra}`;
test('healthy Home has no attribution footer, footer RPC, branding fetch or inert subtitle controls',async({page})=>{
 const s=await maintenanceServer();try{
  const wire=await publicWire(page,s.origin,{libraries:true});
  await page.goto(s.origin);
  await expect(page.getByRole('heading',{name:'Home',exact:true})).toBeVisible();
  await expect(page.getByText('Loading Screenroom…',{exact:true})).toHaveCount(0);
  await page.getByRole('button',{name:'Expand sidebar',exact:true}).click();
  await expect(page.locator('footer, .provider-attribution')).toHaveCount(0);
  await expect(page.getByRole('group',{name:'Subtitles',exact:true})).toHaveCount(0);
  await expect(page.getByRole('radio')).toHaveCount(0);
  expect(wire.calls).toContain('GetHome');
  expect(wire.calls).not.toContain('GetCatalogInfo');
  expect(wire.calls).not.toContain('ListArtwork');
 }finally{await s.close();}
});
test('semantic-only read failure does not fabricate lexical results or successful effective methods',async({page})=>{
 const s=await maintenanceServer();try{
  const calls=await consumerWire(page,s.origin,c=>c.method==='Search'?false:undefined);
  await page.goto(s.origin+scoped('search','&q=semantic-only&methods=semantic'));
  await expect(page.getByRole('alert')).toBeVisible();await expect(page.getByRole('button',{name:'Retry search',exact:true})).toBeEnabled();
  const searches=calls.filter(c=>c.method==='Search');expect(searches).toHaveLength(1);expect([...nested(searches[0],4).keys()]).toEqual([4]);
  await expect(page.locator('[aria-label="Search results"]')).toHaveCount(0);await expect(page.getByRole('status').filter({hasText:'Effective methods'})).toHaveCount(0);assertScopes(calls);
 }finally{await s.close();}
});
const searchResponse=(title:string,degradation='')=>msg(text(1,item(title)),text(3,msg(bit(2))),...(degradation?[text(4,degradation)]:[]));
async function refresh(page:Page){const expand=page.getByRole('button',{name:'Expand sidebar',exact:true});if(await expand.count())await expand.click();await page.getByRole('button',{name:'Refresh',exact:true}).click();}
function assertScopes(calls:Call[]){const relevant=calls.filter(c=>['GetItem','ListChildren','ListCredits','Search','ListLibraryItems'].includes(c.method));expect(relevant.length).toBeGreaterThan(0);for(const c of relevant){if(c.method==='Search'){expect(value(c,7)).toBe(source);expect(value(c,2)).toBe(library);}else if(c.method==='ListLibraryItems'){expect(value(c,3)).toBe(source);expect(value(c,1)).toBe(library);}else{const scope=nested(c,c.method==='GetItem'?2:3);expect(scope.get(1)?.toString()).toBe(source);expect(scope.get(2)?.toString()).toBe(library);}}}
test('search paging and detail return retain query, methods, source and library in browser history',async({page})=>{
 const s=await maintenanceServer();try{
  const calls=await consumerWire(page,s.origin,c=>{
   if(c.method==='Search')return msg(searchResponse(token(c,6)?'second-result':'first-result'),...(!token(c,6)?[text(2,'search-next')]:[]));
   if(c.method==='GetItem')return text(1,item(value(c,1)));
  });
  await page.goto(s.origin+scoped('search','&q=history-query&methods=text&types=Movie'));
  await expect(page.locator('[aria-label="Search results"] a')).toHaveCount(2);
  expect(calls.filter(c=>c.method==='Search').map(c=>token(c,6))).toEqual(['','search-next']);
  await page.locator('[aria-label="Search results"] a').filter({hasText:'second-result'}).click();
  await expect(page.getByRole('heading',{name:'second-result',exact:true})).toBeVisible();
  await page.goBack();await expect(page.getByRole('searchbox')).toHaveValue('history-query');
  await expect(page.locator('[aria-label="Search results"]')).toContainText('first-result');
  const url=new URL(page.url());expect(url.searchParams.get('methods')).toBe('text');expect(url.searchParams.get('source_id')).toBe(source);expect(url.searchParams.get('library')).toBe(library);
  const searches=calls.filter(c=>c.method==='Search');expect(searches.length).toBeGreaterThanOrEqual(2);for(const c of searches){expect(value(c,1)).toBe('history-query');expect([...nested(c,4).keys()]).toEqual([2]);}
  expect(calls.filter(c=>c.method==='ListCredits').map(c=>value(c,1))).toEqual(['second-result']);assertScopes(calls);
 }finally{await s.close();}
});
for(const method of ['ListChildren','ListCredits'])test(`${method} repeated tokens fail visibly instead of silently looping`,async({page})=>{
 const s=await maintenanceServer();try{
  const calls=await consumerWire(page,s.origin,c=>{
   if(c.method==='GetItem')return text(1,item(value(c,1),value(c,1)==='series'?'series':'movie'));
   if(c.method===method)return msg(text(1,method==='ListChildren'?item(token(c,2)?'second-season':'first-season','season','series'):msg(text(1,token(c,2)?'second-person':'first-person'),text(2,'Protocol person'),text(4,'cast'))),text(2,'repeated-token'));
  });
  await page.goto(s.origin+scoped('browse',method==='ListChildren'?'&series=series':'&title=detail'));
  await expect(page.getByRole('alert')).toBeVisible();
  const reads=calls.filter(c=>c.method===method);expect(reads.map(c=>token(c,2))).toEqual(['','repeated-token']);
  expect(reads.map(c=>value(c,1))).toEqual(method==='ListChildren'?['series','series']:['detail','detail']);
  await expect(page.getByText('Loading Screenroom…',{exact:true})).toHaveCount(0);assertScopes(calls);
 }finally{await s.close();}
});
test('old delayed search cannot overwrite newer results or effective-method status',async({page})=>{
 const s=await maintenanceServer();let release=()=>{};
 try{
  const calls=await consumerWire(page,s.origin,async c=>{if(c.method!=='Search')return;if(value(c,1)==='old')await new Promise<void>(r=>release=r);return searchResponse(value(c,1),value(c,1)==='old'?'embedding_incompatible':'embedding_unavailable');});
  await page.goto(s.origin+scoped('search','&q=old'));await expect.poll(()=>calls.filter(c=>c.method==='Search').length).toBe(1);
  const cancelled=page.waitForEvent('requestfailed',{predicate:request=>{
   if(!request.url().endsWith('/Search'))return false;
   const body=request.postDataBuffer();return !!body&&decode(body.subarray(5,5+body.readUInt32BE(1))).get(1)?.toString()==='old';
  }});
  await page.getByRole('searchbox').fill('new');
  const oldRequest=await cancelled;expect(oldRequest.failure()?.errorText).toBe('net::ERR_ABORTED');
  await expect(page.locator('[aria-label="Search results"]')).toContainText('new');
  release();
  // The old transport is demonstrably aborted, not presumed delivered by route.fulfill.
  await page.evaluate(()=>new Promise<void>(resolve=>requestAnimationFrame(()=>requestAnimationFrame(()=>resolve()))));
  await expect(page.getByRole('status').filter({hasText:'Effective methods'})).toHaveText('Effective methods: text. embedding_unavailable');
  await expect(page.locator('[aria-label="Search results"]')).toContainText('new');
  await expect(page.locator('[aria-label="Search results"]')).not.toContainText('old');
  expect(calls.filter(c=>c.method==='Search').map(c=>value(c,1))).toEqual(['old','new']);assertScopes(calls);
 }finally{release();await s.close();}
});
for(const kind of ['search','detail'])test(`${kind} Refresh issues fresh RPCs, settles successful and failed reads, and allows retry`,async({page})=>{
 const s=await maintenanceServer();let fail=false,revision=0;
 try{
  const method=kind==='search'?'Search':'GetItem';
  const calls=await consumerWire(page,s.origin,c=>{if(c.method===method){if(fail)return false;return kind==='search'?searchResponse(`revision-${revision}`):text(1,item('detail'));}if(c.method==='ListCredits')return text(1,msg(text(1,`credit-${revision}`),text(2,`Person ${revision}`),text(3,'Role'),text(4,'cast')));});
  await page.goto(s.origin+scoped(kind==='search'?'search':'browse',kind==='search'?'&q=test':'&title=detail'));
  await expect(page.getByRole('heading',{name:kind==='search'?'Search':'detail',exact:true})).toBeVisible();
  if(kind==='search')await expect(page.locator('[aria-label="Search results"]')).toContainText('revision-0');
  const before=calls.filter(c=>c.method===method).length;revision=1;await refresh(page);
  await expect.poll(()=>calls.filter(c=>c.method===method).length).toBeGreaterThan(before);
  await expect(page.getByText('Refreshed from server.',{exact:true})).toBeVisible();
  if(kind==='search')await expect(page.locator('[aria-label="Search results"]')).toContainText('revision-1');else await expect(page.getByText('Person 1',{exact:true})).toBeVisible();
  fail=true;await refresh(page);await expect(page.getByRole('alert')).toBeVisible();
  await expect(page.locator(kind==='search'?'[aria-label="Search results"]':'article.detail')).toHaveCount(0);
  await expect(page.getByText('Refreshing…',{exact:true})).toHaveCount(0);await expect(page.getByText('Refreshed from server.',{exact:true})).toHaveCount(0);
  fail=false;const failedCount=calls.filter(c=>c.method===method).length;
  await page.getByRole('button',{name:kind==='search'?'Retry search':'Retry',exact:true}).click();
  await expect.poll(()=>calls.filter(c=>c.method===method).length).toBeGreaterThan(failedCount);await expect(page.getByRole('alert')).toHaveCount(0);assertScopes(calls);
  if(kind==='search')await expect(page.locator('[aria-label="Search results"]')).toContainText('revision-1');else await expect(page.getByRole('heading',{name:'detail',exact:true})).toBeVisible();
 }finally{await s.close();}
});
test('source scope survives hierarchy, episode and credit pagination, detail return and browser history',async({page})=>{
 const s=await maintenanceServer();try{
  const calls=await consumerWire(page,s.origin,c=>{
   const id=value(c,1);
   if(c.method==='GetItem')return text(1,item(id,id==='series'?'series':id==='season'?'season':'episode',id==='season'?'series':id.startsWith('episode')?'season':''));
   if(c.method==='ListLibraryItems')return text(1,item('series','series'));
   if(c.method==='ListChildren'){if(id==='series')return text(1,item('season','season','series'));return msg(text(1,item(token(c,2)?'episode-2':'episode-1','episode','season')),...(!token(c,2)?[text(2,'children-next')]:[]));}
   if(c.method==='ListCredits')return msg(text(1,msg(text(1,token(c,2)?'person-2':'person-1'),text(2,token(c,2)?'Person two':'Person one'),text(3,'Role'),text(4,'cast'))),...(!token(c,2)?[text(2,'credits-next')]:[]));
  });
  await page.goto(s.origin+scoped('browse'));await page.locator('[aria-label="Catalog"] a').filter({hasText:'series'}).click();
  await page.locator('[aria-label="Catalog"] a').filter({hasText:'season'}).click();await expect(page).toHaveURL(/season=season/);
  await expect(page.locator('[aria-label="Catalog"] a')).toHaveCount(2);
  expect(calls.filter(c=>c.method==='ListChildren'&&value(c,1)==='season').map(c=>token(c,2))).toEqual(['','children-next']);
  await page.locator('[aria-label="Catalog"] a').filter({hasText:'episode-2'}).click();
  await expect(page.getByRole('heading',{name:'episode-2',exact:true})).toBeVisible();await expect(page.getByText('Person two',{exact:true})).toBeVisible();
  expect(calls.filter(c=>c.method==='ListCredits').map(c=>token(c,2))).toEqual(['','credits-next']);
  expect(calls.filter(c=>c.method==='ListCredits').map(c=>value(c,1))).toEqual(['episode-2','episode-2']);
  await page.goBack();await expect(page.locator('[aria-label="Catalog"]')).toBeVisible();await expect(page).toHaveURL(/season=season/);
  await page.goForward();await expect(page.getByRole('heading',{name:'episode-2',exact:true})).toBeVisible();assertScopes(calls);
 }finally{await s.close();}
});
test('foreign episode ancestry is not adopted as the requested season detail',async({page})=>{const s=await maintenanceServer();try{const calls=await consumerWire(page,s.origin,c=>{if(c.method==='GetItem'){const id=value(c,1);return text(1,item(id,id==='series'?'series':id==='season'||id==='foreign-season'?'season':'episode',id==='season'?'series':id==='foreign-episode'?'foreign-season':id==='foreign-season'?'other-series':''));}});await page.goto(s.origin+scoped('browse','&series=series&season=season&title=foreign-episode'));await expect(page.getByText('0 loaded',{exact:true})).toBeVisible();await expect(page.getByRole('heading',{name:'foreign-episode',exact:true})).toHaveCount(0);expect(calls.filter(c=>c.method==='ListCredits')).toHaveLength(0);expect(calls.filter(c=>c.method==='GetItem').map(c=>value(c,1))).toContain('foreign-season');assertScopes(calls);}finally{await s.close();}});
for(const semantic of [true,false])test(`search capability filtering and explicit ${semantic?'semantic degradation':'unsupported semantic removal'}`,async({page})=>{const s=await maintenanceServer();try{const calls=await consumerWire(page,s.origin,c=>c.method==='Search'?searchResponse('result',semantic?'embedding_coverage_incomplete':''):undefined,semantic);await page.goto(s.origin+scoped('search','&q=method-test&methods=text,semantic'));await expect(page.locator('[aria-label="Search results"]')).toContainText('result');const searches=calls.filter(c=>c.method==='Search');expect(searches.length).toBeGreaterThan(0);expect(nested(searches.at(-1)!,4).has(2)).toBe(true);expect(nested(searches.at(-1)!,4).has(4)).toBe(semantic);await expect(page.getByRole('status').filter({hasText:'Effective methods'})).toContainText('Effective methods: text.');if(semantic)await expect(page.getByRole('status').filter({hasText:'Effective methods'})).toContainText('embedding_coverage_incomplete');else{await page.getByRole('button',{name:'Search settings',exact:true}).click();await expect(page.getByRole('checkbox',{name:'Semantic',exact:true})).toHaveCount(0);}assertScopes(calls);}finally{await s.close();}});
