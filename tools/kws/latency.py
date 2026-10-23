import numpy as np, soundfile as sf, sherpa_onnx, sys
D="./sherpa-onnx-kws-zipformer-zh-en-3M-2025-12-20/"
def run(chunk, thr=0.35):
    sp=sherpa_onnx.KeywordSpotter(encoder=D+f"encoder-epoch-13-avg-2-chunk-{chunk}-left-64.int8.onnx",
      decoder=D+f"decoder-epoch-13-avg-2-chunk-{chunk}-left-64.onnx",
      joiner=D+f"joiner-epoch-13-avg-2-chunk-{chunk}-left-64.int8.onnx",
      tokens=D+"tokens.txt",keywords_file=D+"keywords.txt",num_threads=1,
      keywords_score=1.0,keywords_threshold=thr)
    d,sr=sf.read("/home/wings/BrainWorkshop/.tmp/tts/t/short1.mp3",dtype='float32')
    n=int(len(d)*16000/sr); d=d[np.minimum((np.arange(n)*(sr/16000)).astype(int),len(d)-1)]
    s=sp.create_stream()
    for i in range(0,len(d),800):
        s.accept_waveform(16000,d[i:i+800])
        while sp.is_ready(s):
            sp.decode_stream(s); r=sp.get_result(s)
            if r: return round(i/16000,2)
    return None
for c in [8,16]:
    print(f"chunk={c}: 触发时刻 {run(c)}s （音频为单独“小蝶”，词尾约 0.55s）")
