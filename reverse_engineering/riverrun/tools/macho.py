import struct,sys,subprocess
class MachO:
    def __init__(s,path):
        s.d=open(path,'rb').read()
        magic,cpu,sub,ft,ncmds,sz,fl=struct.unpack_from('<7I',s.d,0)
        assert magic==0xfeedface
        off=28; s.segs=[]
        for _ in range(ncmds):
            cmd,csz=struct.unpack_from('<2I',s.d,off)
            if cmd==1:
                name=s.d[off+8:off+24].rstrip(b'\0').decode()
                va,vs,fo,fs=struct.unpack_from('<4I',s.d,off+24)
                s.segs.append((name,va,vs,fo,fs))
            off+=csz
        s.syms={}
        for l in subprocess.run(['nm','-n',path],capture_output=True,text=True).stdout.splitlines():
            p=l.split()
            if len(p)>=3: s.syms[p[2]]=int(p[0],16)
        dm=subprocess.run(['nm','-n','-C',path],capture_output=True,text=True).stdout.splitlines()
        s.dsyms={}
        for l in dm:
            p=l.split(' ',2)
            if len(p)==3 and p[0].strip(): s.dsyms[p[2]]=int(p[0],16)
    def read(s,va,n):
        for name,sva,vs,fo,fs in s.segs:
            if sva<=va<sva+vs:
                o=va-sva
                if o>=fs: return b'\0'*n
                return s.d[fo+o:fo+o+n]
        raise KeyError(hex(va))
    def f32(s,va,n=1): return struct.unpack('<%df'%n,s.read(va,4*n))
    def f64(s,va,n=1): return struct.unpack('<%dd'%n,s.read(va,8*n))
    def u32(s,va,n=1): return struct.unpack('<%dI'%n,s.read(va,4*n))
    def i32(s,va,n=1): return struct.unpack('<%di'%n,s.read(va,4*n))
