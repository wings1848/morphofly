import sys, numpy as np, soundfile as sf, sherpa_onnx
D="./sherpa-onnx-kws-zipformer-zh-en-3M-2025-12-20/"
def mk(enc,joi,thr):
    return sherpa_onnx.KeywordSpotter(encoder=D+enc,decoder=D+"decoder-epoch-13-avg-2-chunk-16-left-64.onnx",
        joiner=D+joi,tokens=D+"tokens.txt",keywords_file=D+"keywords.txt",
        num_threads=1,keywords_score=1.0,keywords_threshold=thr)
def run(sp,path,snr=None):
    d,sr=sf.read(path,dtype='float32')
    if d.ndim>1: d=d[:,0]
    if sr!=16000:
        n=int(len(d)*16000/sr); d=d[np.minimum((np.arange(n)*(sr/16000)).astype(int),len(d)-1)]
    if snr is not None:
        rms=np.sqrt((d**2).mean()); np.random.seed(0)
        noise=np.random.randn(len(d)).astype('float32')
        noise*=rms/ (10**(snr/20)) / np.sqrt((noise**2).mean()+1e-12)
        d=(d+noise).astype('float32')
    s=sp.create_stream(); n=int(16000*0.1); hits=[]
    for i in range(0,len(d),n):
        s.accept_waveform(16000,d[i:i+n])
        while sp.is_ready(s):
            sp.decode_stream(s); r=sp.get_result(s)
            if r: hits.append(r); sp.reset_stream(s)
    return hits
pos=["pos_xx","pos_xy","pos_yx","pos_yy","pos_xb","pos_hc"]
neg=["neg1","neg2","n3","n4","n5","n6","fp_die","fp_xiaode","fp_xiaodi","fp_xiaoli","fp_hudie2"]
for enc,joi,name in [("encoder-epoch-13-avg-2-chunk-16-left-64.int8.onnx","joiner-epoch-13-avg-2-chunk-16-left-64.int8.onnx","int8")]:
  for thr in [0.25,0.35]:
    sp=mk(enc,joi,thr)
    for snr in [None,15,5]:
        p=sum(1 for f in pos if run(sp,f"/home/wings/BrainWorkshop/.tmp/tts/t/{f}.mp3",snr))
        f_n=sum(1 for f in neg if run(sp,f"/home/wings/BrainWorkshop/.tmp/tts/t/{f}.mp3",snr))
        print(f"{name} thr={thr} snr={snr}: 命中 {p}/{len(pos)}  误报 {f_n}/{len(neg)}")
