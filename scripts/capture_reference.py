#!/usr/bin/env -S uv run
"""Capture calls made by the existing reference tests without altering production logic."""

import json
import os
import pathlib
import re
import shutil
import subprocess

ROOT = pathlib.Path(__file__).resolve().parents[1]
SOURCE = ROOT / "target/validation/reference"
DEST = ROOT / "target/validation/capture"
DEST.mkdir(parents=True, exist_ok=True)
for source in SOURCE.glob("*.go"):
    shutil.copy2(source, DEST / source.name)
for name in ("go.mod", "go.sum"):
    shutil.copy2(SOURCE / name, DEST / name)

# Each wrapper calls the unchanged production function, records its complete
# arguments and return value, and returns the original result to its assertions.
specs = {
    "matchWildcard": (
        "pattern, text string",
        "bool",
        "pattern, text",
        'map[string]any{"pattern":pattern,"text":text}',
        "value",
    ),
    "matchAnyPattern": (
        "patterns []string, text string",
        "bool",
        "patterns, text",
        'map[string]any{"patterns":patterns,"text":text}',
        "value",
    ),
    "parseCommaList": ("text string", "[]string", "text", "text", "value"),
    "formatKey": ("text string", "string", "text", "text", "value"),
    "shortenTitle": ("text string", "string", "text", "text", "value"),
    "shortHash": ("text string", "string", "text", "text", "value"),
    "generatePRBody": (
        "commit *Commit, body, stack string",
        "string",
        "commit, body, stack",
        'map[string]any{"commit":commit,"body":body,"stack":stack,"template":getPRTemplate()}',
        "value",
    ),
    "generateStackInfo": (
        "commits []*Commit, current *Commit, history []PRHistoryEntry, merged map[int]bool",
        "string",
        "commits,current,history,merged",
        'map[string]any{"commits":commits,"current":current,"history":history,"merged":merged,"reverse":config.reverse,"emoji":!config.stackEmojiDisabled,"host":config.git.host,"repo":config.git.repo}',
        "value",
    ),
    "extractPRHistoryFromStackInfo": (
        "body string",
        "[]PRHistoryEntry",
        "body",
        "body",
        "value",
    ),
    "extractPRNumbersFromStackInfo": ("body string", "[]int", "body", "body", "value"),
    "formatPROutput": (
        "template string, commit *Commit",
        "string",
        "template,commit",
        'map[string]any{"template":template,"commit":commit,"host":config.git.host,"repo":config.git.repo}',
        "value",
    ),
    "dedupStrings": ("values []string", "[]string", "values", "values", "value"),
    "selectPRNumberForHeadRef": (
        "prs []PR, reference string",
        "int",
        "prs,reference",
        'map[string]any{"prs":prs,"reference":reference}',
        "value",
    ),
    "selectLocalBranchForCommit": (
        "lines []string, commit *Commit",
        "string",
        "lines,commit",
        'map[string]any{"lines":lines,"commit":commit}',
        "value",
    ),
    "remoteRefMatches": (
        "output, reference, hash string",
        "bool",
        "output,reference,hash",
        'map[string]any{"output":output,"reference":reference,"hash":hash}',
        "value",
    ),
    "decideRefSync": (
        "local,pushed string, ff,same bool",
        "refSyncAction",
        "local,pushed,ff,same",
        'map[string]any{"local":local,"pushed":pushed,"ff":ff,"same":same}',
        "value",
    ),
    "ghStackReason": ("output string", "string", "output", "output", "value"),
    "isStackPartiallyUnstacked": ("output string", "bool", "output", "output", "value"),
    "isBaseChangeBlockedByStack": (
        "err error",
        "bool",
        "err",
        "errorText(err)",
        "value",
    ),
    "isGhStackMissing": (
        "output string, err error",
        "bool",
        "output,err",
        'map[string]any{"output":output,"error":errorText(err)}',
        "value",
    ),
    "isStackNeedsRebuild": (
        "output string, err error",
        "bool",
        "output,err",
        'map[string]any{"output":output,"error":errorText(err)}',
        "value",
    ),
    "isStackPushRejected": (
        "output string, err error",
        "bool",
        "output,err",
        'map[string]any{"output":output,"error":errorText(err)}',
        "value",
    ),
}
helpers = """package main
import("encoding/json";"os";"testing";"sync";"fmt")
var captureMutex sync.Mutex
func errorText(err error) string { if err==nil{return ""};return err.Error() }
func recordCall(t *testing.T,name string,input,output any){ t.Helper();captureMutex.Lock();defer captureMutex.Unlock(); f,err:=os.OpenFile(os.Getenv("GIT_PR_CAPTURE"),os.O_CREATE|os.O_WRONLY|os.O_APPEND,0600);if err!=nil{t.Fatal(err)};defer f.Close();if err=json.NewEncoder(f).Encode(map[string]any{"case":t.Name(),"function":name,"input":input,"output":output});err!=nil{t.Fatal(err)} }
"""
for name, (decl, result, args, data, _) in specs.items():
    helpers += f'func capture_{name}(t *testing.T,{decl}) {result} {{ value:={name}({args});recordCall(t,"{name}",{data},value);return value }}\n'
helpers += """
func capture_parseLogs(t *testing.T, input string)(CommitList,error){ v,e:=parseLogs(input);recordCall(t,"parseLogs",input,map[string]any{"value":v,"error":errorText(e)});return v,e }
func capture_parseTrailers(t *testing.T,input []string)(string,[]KeyVal){m,a:=parseTrailers(input);recordCall(t,"parseTrailers",input,map[string]any{"message":m,"attrs":a});return m,a}
func capture_parseJJWorkingCopy(t *testing.T,check,info string)(*Commit,error){v,e:=parseJJWorkingCopy(check,info);recordCall(t,"parseJJWorkingCopy",map[string]any{"check":check,"info":info,"user":config.git.user,"email":config.git.email},map[string]any{"value":v,"error":errorText(e)});return v,e}
func capture_resolveOutputFormat(t *testing.T,input string)(string,error){v,e:=resolveOutputFormat(input);recordCall(t,"resolveOutputFormat",input,map[string]any{"value":v,"error":errorText(e)});return v,e}
func capture_fullMessage(t *testing.T,commit *Commit) string {input:=*commit; input.Attrs=append([]KeyVal{},commit.Attrs...);v:=commit.FullMessage();recordCall(t,"FullMessage",input,v);return v}
func capture_orderSelection(t *testing.T,stack []*Commit,selected []string)(int,int,error){lo,hi,e:=orderSelection(stack,selected);recordCall(t,"orderSelection",map[string]any{"stack":stack,"selected":selected},map[string]any{"lo":lo,"hi":hi,"error":errorText(e)});return lo,hi,e}
func capture_findPrevNonSkipped(t *testing.T,stack []*Commit,target *Commit)*Commit{v:=findPrevNonSkipped(stack,target);recordCall(t,"findPrevNonSkipped",map[string]any{"stack":stack,"target":target},v);return v}
func capture_validateRemoteRefsBeforePush(t *testing.T,commits []*Commit)[]string{v:=validateRemoteRefsBeforePush(commits);recordCall(t,"validateRemoteRefsBeforePush",commits,v);return v}
func capture_pickStackLeaf(t *testing.T,tips []string,ancestor func(string,string)bool)(string,bool){leaf,ok:=pickStackLeaf(tips,ancestor);edges:=map[string]bool{};for _,a:=range tips{for _,b:=range tips{edges[a+"|"+b]=ancestor(a,b)}};recordCall(t,"pickStackLeaf",map[string]any{"tips":tips,"edges":edges},map[string]any{"leaf":leaf,"ok":ok});return leaf,ok}
func capture_resolveGitHubStackEnabled(t *testing.T,no bool,settings ...optionalBool)bool{v:=resolveGitHubStackEnabled(no,settings...);values:=[]any{};for _,s:=range settings{if s.set{values=append(values,s.value)}else{values=append(values,nil)}};recordCall(t,"resolveGitHubStackEnabled",map[string]any{"no":no,"settings":values},v);return v}
func capture_markUnselected(t *testing.T,commits []*Commit,selected map[string]bool,announce bool){b,_:=json.Marshal(commits);var before any;_ = json.Unmarshal(b,&before);markUnselected(commits,selected,announce);recordCall(t,"markUnselected",map[string]any{"commits":before,"selected":selected},commits)}
func capture_recoverRangeTip(t *testing.T,enabled bool,id,head string,depth int,change func(string)(string,error),atDepth func(string,int)(string,error))(string,error){trace:=[]string{};v,e:=recoverRangeTip(enabled,id,head,depth,func(s string)(string,error){trace=append(trace,"change:"+s);return change(s)},func(s string,d int)(string,error){trace=append(trace,fmt.Sprintf("depth:%s:%d",s,d));return atDepth(s,d)});recordCall(t,"recoverRangeTip",map[string]any{"jj":enabled,"id":id,"head":head,"depth":depth},map[string]any{"value":v,"error":errorText(e),"trace":trace});return v,e}
"""
extra = [
    "parseLogs",
    "parseTrailers",
    "parseJJWorkingCopy",
    "resolveOutputFormat",
    "orderSelection",
    "findPrevNonSkipped",
    "validateRemoteRefsBeforePush",
    "pickStackLeaf",
    "resolveGitHubStackEnabled",
    "markUnselected",
    "recoverRangeTip",
]
for path in DEST.glob("*_test.go"):
    text = path.read_text()
    for name in list(specs) + extra:
        text = re.sub(r"\b" + name + r"\(", "capture_" + name + "(t,", text)
    text = re.sub(r"\b(\w+)\.FullMessage\(\)", r"capture_fullMessage(t,\1)", text)
    path.write_text(text)
(DEST / "capture_test.go").write_text(helpers)
output = ROOT / "tests/fixtures/reference.jsonl"
output.parent.mkdir(parents=True, exist_ok=True)
output.unlink(missing_ok=True)
env = dict(
    os.environ,
    GIT_PR_CAPTURE=str(output),
    GOCACHE=str(pathlib.Path.home() / ".cache/git-pr/go-build"),
)
result = subprocess.run(
    ["go", "test", "-count=1", "-json", "./..."],
    cwd=DEST,
    env=env,
    capture_output=True,
    text=True,
)
(ROOT / "target/validation/reference-tests.jsonl").write_text(result.stdout)
if result.returncode:
    print(result.stdout, result.stderr)
    raise SystemExit(result.returncode)
records = [json.loads(line) for line in output.read_text().splitlines()]
records.sort(key=lambda r: (r["case"], r["function"], json.dumps(r["input"], sort_keys=True)))
output.write_text(
    "".join(json.dumps(r, ensure_ascii=False, sort_keys=True) + "\n" for r in records)
)
print(f"Captured {len(records)} calls across {len(set(r['case'] for r in records))} test cases.")
