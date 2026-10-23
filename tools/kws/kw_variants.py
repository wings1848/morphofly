import numpy as np, soundfile as sf, sherpa_onnx, os
D="./sherpa-onnx-kws-zipformer-zh-en-3M-2025-12-20/"
def mk(kwfile, thr):
    return sherpa_onnx.KeywordSpotter(encoder=D+"encoder-epoch-13-avg-2-chunk-16-left-64.int8.onnx",
      decoder=D+"decoder-epoch-13-avg-2-chunk-16-left-64.onnx",
      joiner=D+"joiner-epoch-13-avg-2-chunk-16-left-64.int8.onnx",
      tokens=D+"tokens.txt",keywords_file=kwfile,num_threads=1,
      keywords_score=1.0,keywords_threshold=thr)
def run(sp,path):
    d,sr=sf.read(path,dtype='float32')
    if d.ndim>1:d=d[:,0]
    n=int(len(d)*16000/sr); d=d[np.minimum((np.arange(n)*(sr/16000)).astype(int),len(d)-1)]
    s=sp.create_stream();hits=[]
    for i in range(0,len(d),1600):
        s.accept_waveform(16000,d[i:i+1600])
        while sp.is_ready(s):
            sp.decode_stream(s);r=sp.get_result(s)
            if r: hits.append(r); sp.reset_stream(s)
    return hits
T="/home/wings/BrainWorkshop/.tmp/tts/t/"
pos=["pos_xx","pos_xy","pos_yx","pos_yy","pos_xb","pos_hc","short1"]
neg=["hard7","hard5","neg1","neg2","n3","n4","n5","n6","fp_xiaodi","fp_xiaode","fp_die","fp_hudie2","fp_xiaoli","/tmp/longneg.mp3"]
for name,kw in [("仅小蝶","kw_a.txt"),("仅小蝶小蝶","kw_b.txt"),("两个都收","kw_c.txt")]:

    for thr in [0.25,0.5]:
        sp=mk(kw,thr)
        p=sum(1 for f in pos if run(sp, f if f.startswith("/") else T+f+".mp3"))
        fp=[f for f in neg if run(sp, f if f.startswith("/") else T+f+".mp3")]
        print(f"{name} thr={thr}: 唤醒 {p}/{len(pos)}  误唤醒 {len(fp)}/{len(neg)} {fp}")
