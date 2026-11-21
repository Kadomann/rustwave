"""CW-Dekoder mit multimon-ng vergleichen: Testsignale aus tools/cw_testsignale.py in /tmp/cw, dann python3 tools/cw_vergleich.py"""
import subprocess, difflib, glob, os, numpy as np, wave, scipy.signal as sg, re
ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
TXT="CQ CQ DE DL1ABC DL1ABC PSE K = UR RST 579 579 NAME PETER QTH BERLIN = HW? DL1ABC DE OE3XYZ K"
def acc(got, ref):
    g=re.sub(r"\s+"," ",got).strip(); return difflib.SequenceMatcher(None,g,ref).ratio()
def ours(f, pitch=600):
    r=subprocess.run(["cargo","test","--release","cw_wav","--","--ignored","--nocapture"],cwd=ROOT,
        env=dict(os.environ,CW_WAV=f,CW_PITCH=str(pitch)),capture_output=True,text=True).stdout
    m=re.search(r"TEXT: (.*)",r); return m.group(1) if m else ""
def mm(f):
    w=wave.open(f); x=np.frombuffer(w.readframes(w.getnframes()),np.int16).astype(float)
    y=sg.resample_poly(x,147,320).astype(np.int16); open("/tmp/cw/x.raw","wb").write(y.tobytes())
    r=subprocess.run(["multimon-ng","-q","-t","raw","-a","MORSE_CW","/tmp/cw/x.raw"],capture_output=True,text=True).stdout
    return r.replace("MORSE_CW: ","").replace("\n","")
for f in sorted(glob.glob("/tmp/cw/*.wav")):
    n=os.path.basename(f)[:-4]; ref=TXT+" "+TXT if n=="tempowechsel" else TXT
    p=620 if n=="hand20" else 600
    a=ours(f,p); b=mm(f)
    print(f"{n:14s} wir {acc(a,ref):.2f}   multimon {acc(b,ref):.2f}   | {a[:70]}")
