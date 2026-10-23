import sys, numpy as np, soundfile as sf, sherpa_onnx
d = ""
sp = sherpa_onnx.KeywordSpotter(
    encoder=d+"encoder-epoch-13-avg-2-chunk-16-left-64.int8.onnx",
    decoder=d+"decoder-epoch-13-avg-2-chunk-16-left-64.onnx",
    joiner=d+"joiner-epoch-13-avg-2-chunk-16-left-64.int8.onnx",
    tokens=d+"tokens.txt", keywords_file=d+"keywords.txt",
    num_threads=1, keywords_score=1.0, keywords_threshold=0.1)
samples, sr = sf.read(sys.argv[1], dtype="float32")
if samples.ndim > 1: samples = samples[:, 0]
if sr != 16000:  # linear resample
    t = np.arange(int(len(samples)*16000/sr)) * (sr/16000.0)
    idx = np.minimum(t.astype(int), len(samples)-1)
    samples = samples[idx]
s = sp.create_stream(); n = int(16000*0.1); hits=[]
for i in range(0, len(samples), n):
    s.accept_waveform(16000, samples[i:i+n])
    while sp.is_ready(s):
        sp.decode_stream(s); r = sp.get_result(s)
        if r: hits.append((i/16000, r)); sp.reset_stream(s)
print(f"{sys.argv[1]}: dur={len(samples)/16000:.2f}s hits={len(hits)}")
for h in hits: print("   HIT", h)
