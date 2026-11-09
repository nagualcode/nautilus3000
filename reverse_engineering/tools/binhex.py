import sys,struct
ALPH=b"!\"#$%&'()*+,-012345689@ABCDEFGHIJKLMNPQRSTUVXYZ[`abcdefhijklmpqr"
def decode(path,outdir):
    t=open(path,'rb').read()
    i=t.index(b'\n:')+2 if b'\n:' in t else t.index(b':')+1
    j=t.index(b':',i)
    enc=bytes(c for c in t[i:j] if c not in b'\r\n\t ')
    m={c:k for k,c in enumerate(ALPH)}
    bits=0;n=0;raw=bytearray()
    for c in enc:
        bits=((bits<<6)|m[c])&0xffff; n+=6
        if n>=8: n-=8; raw.append((bits>>n)&0xff)
    # RLE 0x90
    out=bytearray(); k=0; prev=0
    while k<len(raw):
        c=raw[k]
        if c==0x90:
            cnt=raw[k+1]; k+=2
            if cnt==0: out.append(0x90); prev=0x90
            else: out.extend([prev]*(cnt-1))
        else: out.append(c); prev=c; k+=1
    p=0; nl=out[0]; name=out[1:1+nl].decode('mac_roman'); p=1+nl+1
    typ,cre=out[p:p+4],out[p+4:p+8]; p+=8+2
    dl,rl=struct.unpack('>II',out[p:p+8]); p+=8+2
    data=out[p:p+dl]; p+=dl+2; rsrc=out[p:p+rl]
    import os
    os.makedirs(outdir,exist_ok=True)
    open(os.path.join(outdir,'data.bin'),'wb').write(data)
    open(os.path.join(outdir,'rsrc.bin'),'wb').write(rsrc)
    print(name,typ,cre,dl,rl)
decode(sys.argv[1],sys.argv[2])
