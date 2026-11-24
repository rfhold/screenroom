import http from 'node:http';
import { once } from 'node:events';
import { fileURLToPath } from 'node:url';
import { createServer, preview } from 'vite';
import { spawn } from 'node:child_process';
import { createRequire } from 'node:module';
import { dirname, join } from 'node:path';
import { decodeOperator, operatorFrame, operatorReply } from './operator-wire';
export const sourceId='00000000-0000-4000-8000-000000000001',libraryId='00000000-0000-4000-8000-000000000002',policyId='00000000-0000-4000-8000-000000000003',runId='00000000-0000-4000-8000-000000000004',secondSourceId='00000000-0000-4000-8000-000000000005',jobId='00000000-0000-4000-8000-000000000007';
export const userId='10000000-0000-4000-8000-000000000001';
const date='2026-11-11T00:00:00Z';
const generatedId=(i:number)=>`00000000-0000-4000-8000-${(1000+i).toString(16).padStart(12,'0')}`;
async function listen(server:http.Server){server.listen(0,'127.0.0.1');await once(server,'listening');return `http://127.0.0.1:${(server.address() as import('node:net').AddressInfo).port}`;}
export type Options={empty?:boolean;missingUser?:boolean;discoveryError?:string;saveError?:string;ambiguousSave?:boolean;delayedDiscovery?:boolean;delayedSave?:boolean;manyLibraries?:boolean;sourceBusy?:boolean;delayedActions?:boolean;delayedLibraries?:boolean;paginated?:boolean;cyclicLibraries?:boolean;productionPreview?:boolean;presentationCounts?:boolean;indexing?:boolean;indexingConflict?:boolean;indexingUnsupported?:boolean;indexingHistory?:boolean;delayedIndexing?:boolean;delayedConfigure?:boolean;indexingReadFailure?:boolean;noIndexingPolicy?:boolean;indexingFailures?:number;noRuns?:boolean};
export async function maintenanceServer(enabled=true,options:Options={}){
 const calls:ReturnType<typeof decodeOperator>[]=[];
 let configured=!options.noIndexingPolicy,connected=!options.empty,hasRuns=!options.noRuns&&!options.noIndexingPolicy&&!options.empty;
 let policy={id:policyId,source_id:sourceId,version:9007199254740993n,enabled:true,automatic_runs:false,next_due_at:date,configuration:{indexing:1,enabled:true,automatic_runs:false,cadence_seconds:86400,reconcile_seconds:604800,overlap_seconds:300,source_concurrency:1,mode:'incremental',components:{metadata:true,artwork:true,embeddings:false},strategy:'all',stale_after_seconds:null,scope:{library_ids:[libraryId],item_kinds:['movie','series'],item_ids:[]},limits:{work_requests:100,work_records:200,lifetime_requests:100000,lifetime_records:100000,deadline_seconds:86400,work_timeout_seconds:120,retry:{max_attempts:3,base_seconds:2,cap_seconds:60,jitter_percent:20}},artwork:{posters:{skip_existing:true}}}};
 const run={id:runId,source_id:sourceId,policy_id:policyId,policy_version:9007199254740993n,state:'complete',items:0,requests:0,started_at:date,finished_at:date};
 const detail={run,observed_records:null,terminal_reason:'budget_exhausted',enumeration_complete:true,work_settled:false,work_succeeded:false,coverage:{unresolved_attempts:2,primary_unique:0,followup_ready:0,followup_delayed:0,followup_leased:0,followup_failed:0,unknown_attempts:9,retryable_attempts:4,permanent_attempts:3,omissions:0,unresolved_operations:0,pending_artwork_publications:0},queue:{ready:0,delayed:0,leased:0,failed:options.presentationCounts?1:0},candidates:{selected:2,checked:1,skipped_ready:0,staged:0,unavailable:options.presentationCounts?1:0,failed:options.presentationCounts?1:0},allowed_actions:['pause','cancel'],all_complete:false,as_of:date};
 let job={id:jobId,run_id:runId,kind:'ingest',state:'failed',available_at:date,lease_until:null,attempts:3,max_attempts:3,generation:0,redrive_allowed:true,redrive_reason:null as string|null};
 const connection={id:generatedId(10),source_id:sourceId,source_key:'test-server',display_name:'Test Jellyfin server',server_version:'10.11.0',state:'ready'};
 const receipts=new Map<string,unknown>(),attempts=new Map<string,number>();
 let releaseAction=()=>{},releaseLibraries=()=>{},releaseIndexing=()=>{},releaseDiscovery=()=>{},releaseSave=()=>{};
 const upstream=http.createServer(async(req,res)=>{
  const chunks=[];for await(const chunk of req)chunks.push(chunk);
  const call=decodeOperator(req.url!,Buffer.concat(chunks));
  const {rpc,body,after}=call;
  calls.push(call);res.setHeader('content-type','application/grpc-web+proto');
  const send=(v:unknown)=>res.end(operatorReply(JSON.stringify(v,(_,x)=>typeof x==='bigint'?`__bigint_${x}`:x).replace(/"__bigint_(\d+)"/g,'$1')));
  const fail=(code:string,status=409)=>{
   const grpc=status===400?3:status===404?5:status===503?14:code==='source_busy'?10:code==='stale'?9:6;
   const detail=code==='source_busy'?`x-screenroom-run-id: ${runId}\r\n`:'';
   return res.end(operatorFrame(128,Buffer.from(`grpc-status: ${grpc}\r\ngrpc-message: ${code}\r\n${detail}`)));
  };
  if(!enabled)return fail('unavailable',503);
  const input=body?JSON.parse(body):null,selected=call.sourceId===secondSourceId?secondSourceId:sourceId;
  const status=()=>({...detail,run:{...run,source_id:selected}});
  const receipt=(name:string,value:()=>unknown,failures=options.indexingFailures||0)=>{const key=`${name}:${input.request_id}`;if(!receipts.has(key))receipts.set(key,structuredClone(value()));const count=(attempts.get(key)||0)+1;attempts.set(key,count);return count<=failures?fail('unavailable',503):send(receipts.get(key));};
  if(rpc==='DiscoverConnection'){
   if(options.delayedDiscovery)await new Promise<void>(resolve=>releaseDiscovery=resolve);
   if(options.discoveryError)return fail(options.discoveryError,options.discoveryError==='discovery_unavailable'?503:400);
   return send({server:{id:'test-server',name:'Test Jellyfin server',version:'10.11.0'},profiles:[{id:'profile-one',name:'Library owner',hidden:false},{id:'hidden-profile',name:'Hidden profile',hidden:true}],libraries:[...Array.from({length:options.manyLibraries?33:2},(_,i)=>({id:`library-${i}`,name:`Library ${i+1}`,kind:i===1?'television':'movies',supported:true})),{id:'collection-one',name:'Collections one',kind:'collections',supported:true},{id:'collection-two',name:'Collections two',kind:'collections',supported:true},{id:'music',name:'Music',kind:'unsupported',supported:false}]});
  }
  if(rpc==='SaveConnection'){
   if(options.delayedSave)await new Promise<void>(resolve=>releaseSave=resolve);
   if(options.saveError)return fail(options.saveError,['server_identity_mismatch','request_conflict','request_expired','server_already_connected'].includes(options.saveError)?409:400);
   return receipt('save',()=>{connected=true;configured=true;hasRuns=input.refresh;return {connection,policy_id:policyId,policy_version:policy.version,run_id:input.refresh?runId:null};},options.ambiguousSave?1:0);
  }
  if(rpc==='ListConnections')return send({items:connected?[connection]:[],next_after:null});
  if(rpc==='ListSources')return send({items:connected?[{id:sourceId,source_key:'Test Jellyfin server',provider:'jellyfin',state:'ready'},{id:secondSourceId,source_key:'Second test server',provider:'jellyfin',state:'ready'}]:[],next_after:null});
  if(rpc==='ListLibraries'){if(options.delayedLibraries&&selected===sourceId)await new Promise<void>(resolve=>releaseLibraries=resolve);return send({items:Array.from({length:options.cyclicLibraries?100:1},(_,i)=>({public_id:i?generatedId(i):libraryId,title:selected===sourceId?'Test library':'Second library'})),next_after:options.cyclicLibraries?(after==='cycle-a'?'cycle-b':'cycle-a'):null});}
  if(rpc==='ConfigurePolicy'||rpc==='ListPolicies'){
   if(rpc==='ConfigurePolicy'){if(options.delayedConfigure)await new Promise<void>(resolve=>releaseIndexing=resolve);if(options.indexingConflict)return fail('stale');if(options.indexingUnsupported)return fail('invalid',400);return receipt('configure',()=>{configured=true;policy={...policy,version:policy.version+1n,configuration:input.configuration,enabled:input.configuration.enabled,automatic_runs:input.configuration.automatic_runs};return {...policy,source_id:selected};});}
   if(options.indexingReadFailure)return fail('unavailable',503);return send({items:configured?[{...policy,source_id:selected}]:[],next_after:null});
  }
  if(rpc==='GetProgress')return send({policy_version:policy.version,watermark:null,next_reconcile_at:date});
  if(rpc==='StartRun'){if(options.sourceBusy)return fail('source_busy');return receipt('start',()=>{hasRuns=true;return {...run,source_id:selected,policy_version:policy.version};});}
  if(rpc==='ListRuns')return send({items:hasRuns?Array.from({length:options.paginated&&!after?100:1},(_,i)=>({...run,source_id:selected,id:after?generatedId(500):i?generatedId(i):runId})):[],next_after:options.paginated&&!after?runId:null});
  if(rpc==='ListCandidates')return send({items:Array.from({length:options.paginated&&!after?100:1},(_,i)=>({source_item_id:generatedId(after?500:i),public_id:generatedId(i),kind:'movie',state:after?'selected':'checked',code:after?null:'ready',selection_reason:null,selected_at:date,updated_at:date})),next_after:options.paginated&&!after?generatedId(99):null});
  if(rpc==='GetExecution')return send({mode:'full',lower_watermark:null,candidate_watermark:date,settled_at:null,fallback_reason:'no_watermark'});
  if(rpc==='ListJobs')return send({items:[job],next_after:null});
  if(rpc==='ListAttempts'){const start=after?BigInt(after)+1n:9007199254740993n,amount=options.indexingHistory&&!after?100:1;return send({items:Array.from({length:amount},(_,i)=>({job_id:jobId,fence:start+BigInt(i),generation:0,started_at:date,finished_at:date,outcome:'retryable',error_code:'timeout'})),next_after:options.indexingHistory&&!after?(start+99n).toString():null});}
  if(rpc==='ListRedrives'){const start=after?Number(after)+1:1,amount=options.indexingHistory&&!after?100:1;return send({items:Array.from({length:amount},(_,i)=>({job_id:jobId,request_id:generatedId(i+start),generation:i+start})),next_after:options.indexingHistory&&!after?String(start+99):null});}
  if(rpc==='RedriveJob')return receipt('redrive',()=>{job={...job,state:'ready',generation:job.generation+1,redrive_allowed:false,redrive_reason:'not_terminal_failed'};return {job_id:jobId,request_id:input.request_id,generation:job.generation};});
  if(rpc==='ObserveQueue'){const relative=input.capabilities===null?null:0;return send({atomic:false,queue_observation:{started_at:date,finished_at:'2026-11-11T00:00:01Z'},blockers_as_of:'2026-11-11T00:00:02Z',effective_capabilities:input.capabilities,worker_liveness:'unknown',queue:{ready:2,delayed:1,leased:0,failed:1,cancelled:0,complete:0,claimable:relative,reclaimable:relative,blocked:relative,exhausted:1,oldest_ready_seconds:42},blockers:{deadline_expired:0,policy_disabled:0,policy_version:0,source_disabled:0,source_changed:0,source_concurrency:1,run_state:0,cooldown:1,capability:relative,attempt_limit:1,next_eligible_at:null,as_of:date},count_semantics:{blocked:'Capability-relative eligibility; categories overlap.',exhausted:'Attempt limit; not success.'}});}
  if(['PauseRun','ResumeRun','CancelRun'].includes(rpc)){if(options.delayedActions)await new Promise<void>(resolve=>releaseAction=resolve);run.state=rpc==='PauseRun'?'paused':rpc==='ResumeRun'?'running':'cancelled';detail.allowed_actions=rpc==='PauseRun'?['resume','cancel']:rpc==='ResumeRun'?['pause','cancel']:[];return send(status());}
  if(rpc==='GetRunStatus'){if(options.delayedIndexing&&selected===sourceId)await new Promise<void>(resolve=>releaseIndexing=resolve);return send(status());}
  return fail('not_found',404);
 });
 const target=await listen(upstream),reservation=http.createServer(),origin=await listen(reservation);await new Promise<void>(resolve=>reservation.close(()=>resolve()));
 process.env.VITE_SCREENROOM_USER_ID=options.missingUser?'':userId;
 process.env.VITE_SCREENROOM_API_ENDPOINT=origin;
 const root=fileURLToPath(new URL('..',import.meta.url));
 try{
  const config={root,configFile:`${root}/vite.config.ts`,clearScreen:false},address={host:'127.0.0.1',port:Number(new URL(origin).port),strictPort:true,proxy:{'/screenroom.operator.v1.OperatorService/':target}};let closeWeb:()=>Promise<void>;
  if(options.productionPreview){const cli=join(dirname(createRequire(import.meta.url).resolve('vite/package.json')),'bin/vite.js');await new Promise<void>((resolve,reject)=>{const child=spawn(process.execPath,[cli,'build','--logLevel','error'],{cwd:root,env:{...process.env},stdio:['ignore','pipe','pipe'],timeout:60000});let output='';child.stdout.on('data',c=>output+=c);child.stderr.on('data',c=>output+=c);child.on('error',reject);child.on('exit',code=>code===0?resolve():reject(new Error(`Build failed: ${output}`)));});const web=await preview({...config,preview:address});const server=web.httpServer as http.Server;closeWeb=async()=>{server.closeAllConnections();await new Promise<void>(resolve=>server.close(()=>resolve()));};}
  else{const web=await createServer({...config,server:address});await web.listen();closeWeb=()=>web.close();}
  return {origin,calls,releaseAction:()=>releaseAction(),releaseLibraries:()=>releaseLibraries(),releaseIndexing:()=>releaseIndexing(),releaseDiscovery:()=>releaseDiscovery(),releaseSave:()=>releaseSave(),close:async()=>{releaseAction();releaseLibraries();releaseIndexing();releaseDiscovery();releaseSave();await closeWeb();upstream.closeAllConnections();await new Promise<void>(resolve=>upstream.close(()=>resolve()));}};
 }catch(e){upstream.closeAllConnections();await new Promise<void>(resolve=>upstream.close(()=>resolve()));throw e;}
}
