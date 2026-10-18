# AM halftone: 45 deg Euclidean dot, supersampled; writes carrot-on-transparent PNG
import sys, numpy as np
from PIL import Image, ImageFilter
src, out = sys.argv[1], sys.argv[2]
cell = float(sys.argv[3]) if len(sys.argv)>3 else 9.0     # dot pitch in output px
outw = int(sys.argv[4]) if len(sys.argv)>4 else 1600
gamma = float(sys.argv[5]) if len(sys.argv)>5 else 1.0
color = tuple(int(sys.argv[6][i:i+2],16) for i in (0,2,4)) if len(sys.argv)>6 else (0xFF,0x75,0x44)
ss = 3
im = Image.open(src).convert('L')
W = outw*ss; H = round(im.height*W/im.width)
im = im.resize((W,H), Image.LANCZOS).filter(ImageFilter.GaussianBlur(cell*ss*0.35))
L = np.asarray(im, dtype=np.float32)/255.0
lo = float(sys.argv[7]) if len(sys.argv)>7 else 0.0; hi = float(sys.argv[8]) if len(sys.argv)>8 else 1.0
L = np.clip((L-lo)/(hi-lo),0,1)
D = np.clip(1.0-L, 0, 1)**gamma            # ink coverage wanted
D[D<0.035]=0                               # paper stays clean
y, x = np.mgrid[0:H, 0:W].astype(np.float32)
c = cell*ss; a = np.deg2rad(45)
u = (x*np.cos(a)+y*np.sin(a))/c; v = (-x*np.sin(a)+y*np.cos(a))/c
u = (u-np.floor(u))*2-1; v = (v-np.floor(v))*2-1   # -1..1 inside cell
au, av = np.abs(u), np.abs(v)
# Euclidean spot: round dots below 50%, inverted round holes above
spot = np.where(au+av<=1, 1-(au*au+av*av)/2, ((au-1)**2+(av-1)**2)/2)
# map spot (1 center .. 0 corner) to threshold rank via empirical CDF so coverage==D
flat = np.sort(spot[:int(c*4)//1*1, :int(c*4)].ravel())[::-1]
rank = np.searchsorted(-flat, -spot)/flat.size
ink = (rank < D).astype(np.float32)
mask = Image.fromarray((ink*255).astype(np.uint8)).resize((outw, round(H/ss)), Image.LANCZOS)
m = np.asarray(mask)
rgba = np.zeros((m.shape[0], m.shape[1], 4), np.uint8); rgba[...,0],rgba[...,1],rgba[...,2]=color; rgba[...,3]=m
Image.fromarray(rgba,'RGBA').save(out, optimize=True)
print(out, rgba.shape)
