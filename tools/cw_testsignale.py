import numpy as np, wave, sys, difflib
M={'A':'.-','B':'-...','C':'-.-.','D':'-..','E':'.','F':'..-.','G':'--.','H':'....','I':'..','J':'.---','K':'-.-','L':'.-..','M':'--','N':'-.','O':'---','P':'.--.','Q':'--.-','R':'.-.','S':'...','T':'-','U':'..-','V':'...-','W':'.--','X':'-..-','Y':'-.--','Z':'--..','0':'-----','1':'.----','2':'..---','3':'...--','4':'....-','5':'.....','6':'-....','7':'--...','8':'---..','9':'----.','/':'-..-.','?':'..--..','=':'-...-'}
fs=48000
def keyed(text,wpm,pitch,jit,rng,amp=0.5):
    dit=1.2/wpm; segs=[(0,0.4)]
    for w in text.split(' '):
        for ci,c in enumerate(w):
            if ci: segs.append((0,3*dit*(1+jit*rng.uniform(-1,1))))
            for k,s in enumerate(M[c]):
                if k: segs.append((0,dit*(1+jit*rng.uniform(-1,1))))
                segs.append((1,(3 if s=='-' else 1)*dit*(1+jit*rng.uniform(-1,1))))
        segs.append((0,7*dit))
    env=np.concatenate([np.full(int(d*fs),float(on)) for on,d in segs])
    k=np.hanning(int(0.008*fs)); k/=k.sum(); env=np.convolve(env,k,'same')  # weiche Flanken
    t=np.arange(len(env))/fs
    return amp*env*np.sin(2*np.pi*pitch*t)
TXT="CQ CQ DE DL1ABC DL1ABC PSE K = UR RST 579 579 NAME PETER QTH BERLIN = HW? DL1ABC DE OE3XYZ K"
def save(n,x):
    w=wave.open(n,'wb');w.setnchannels(1);w.setsampwidth(2);w.setframerate(fs);w.writeframes((np.clip(x,-1,1)*32767).astype(np.int16).tobytes())
rng=np.random.default_rng(int(sys.argv[1]) if len(sys.argv)>1 else 1)
cases={}
for wpm in (15,25,35):
    for snr in (20,10,6,3,0):   # SNR in 500 Hz Bandbreite (dB)
        x=keyed(TXT,wpm,600,0.0,rng)
        # Rauschen: Leistung in 500 Hz -> sigma^2 *500/(fs/2)
        p_sig=0.5**2/2; sig=np.sqrt(p_sig/10**(snr/10)*(fs/2)/500)
        cases[f"w{wpm}_s{snr}"]=x+sig*rng.standard_normal(len(x))
x=keyed(TXT,20,620,0.2,rng); cases["hand20"]=x+0.02*rng.standard_normal(len(x))
x=keyed(TXT,22,600,0.0,rng); t=np.arange(len(x))/fs; cases["qsb"]=x*(0.55+0.45*np.sin(2*np.pi*0.25*t))+0.01*rng.standard_normal(len(x))
x1=keyed(TXT,14,600,0.05,rng); x2=keyed(TXT,32,600,0.0,rng); cases["tempowechsel"]=np.concatenate([x1,x2])+0.01*rng.standard_normal(len(x1)+len(x2))
for k,v in cases.items(): save(k+".wav",v)
