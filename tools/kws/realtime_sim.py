# 模拟实时：每 100ms 喂一次，统计 CPU 占用率
import time, numpy as np, soundfile as sf, sherpa_onnx
D="./sherpa-onnx-kws-zipformer-zh-en-3M-2025-12-20/"
sp=sherpa_onnx.KeywordSpotter(encoder=D+"encoder-epoch-13-avg-2-chunk-16-left-64.int8.onnx",
  decoder=D+"decoder-epoch-13-avg-2-chunk-16-left-64.onnx",
  joiner=D+"joiner-epoch-13-avg-2-chunk-16-left-64.int8.onnx",
  tokens=D+"tokens.txt",keywords_file=D+"keywords.txt",num_threads=1,
  keywords_score=1.0,keywords_threshold=0.35)
d,sr=sf.read("/home/wings/BrainWorkshop/.tmp/tts/t/late.mp3",dtype='float32')
if d.ndim>1: d=d[:,0]
n=int(len(d)*16000/sr); d=d[np.minimum((np.arange(n)*(sr/16000)).astype(int),len(d)-1)]
s=sp.create_stream(); step=1600; proc=0.0; hits=[]
for i in range(0,len(d),step):
    t=time.perf_counter()
    s.accept_waveform(16000,d[i:i+step])
    while sp.is_ready(s):
        sp.decode_stream(s); r=sp.get_result(s)
        if r: hits.append((i/16000,r)); sp.reset_stream(s)
    proc+=time.perf_counter()-t
audio=len(d)/16000
print(f"音频 {audio:.1f}s, 处理耗时 {proc:.2f}s, CPU 单核占用率 ≈ {proc/audio*100:.1f}%")
print("hits",hits)
