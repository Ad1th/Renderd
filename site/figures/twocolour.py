# Crops a halftone PNG and saves it as a 2-colour palette PNG (carrot dots, clear paper)
import sys, numpy as np
from PIL import Image
src, out = sys.argv[1], sys.argv[2]
im = Image.open(src).convert('RGBA')
if len(sys.argv) > 3:
    im = im.crop(tuple(int(n) for n in sys.argv[3].split(',')))
a = np.asarray(im)[..., 3]
rgba = np.zeros(a.shape + (4,), np.uint8)
rgba[..., 0], rgba[..., 1], rgba[..., 2] = 0xFF, 0x75, 0x44
rgba[..., 3] = (a > 127).astype(np.uint8) * 255
Image.fromarray(rgba, 'RGBA').quantize(colors=2, method=Image.Quantize.FASTOCTREE).save(out, optimize=True)
print(out, im.size)
