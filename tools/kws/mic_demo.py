# 实时麦克风 KWS（需 sounddevice + 声卡）
import sys, numpy as np, sherpa_onnx
try:
    import sounddevice as sd
except ImportError:
    print("需要 pip install sounddevice"); sys.exit(1)
D="./sherpa-onnx-kws-zipformer-zh-en-3M-2025-12-20/"
sp=sherpa_onnx.KeywordSpotter(encoder=D+"encoder-epoch-13-avg-2-chunk-16-left-64.int8.onnx",
  decoder=D+"decoder-epoch-13-avg-2-chunk-16-left-64.onnx",
  joiner=D+"joiner-epoch-13-avg-2-chunk-16-left-64.int8.onnx",
  tokens=D+"tokens.txt",keywords_file=D+"keywords.txt",
  num_threads=1,keywords_score=1.0,keywords_threshold=0.35)
s=sp.create_stream()
def cb(indata,frames,time,status):
    s.accept_waveform(16000, indata[:,0].copy())
    while sp.is_ready(s):
        sp.decode_stream(s); r=sp.get_result(s)
        if r: print("唤醒：",r,flush=True); sp.reset_stream(s)
with sd.InputStream(channels=1,samplerate=16000,blocksize=1600,callback=cb):
    print("请说“小蝶”… Ctrl-C 退出",flush=True)
    import time
    while True: time.sleep(1)
