import re,sys
src=open(sys.argv[1]).read()
parts=re.split(r'(?=/\* ==== )',src)
for p in parts:
    m=re.match(r'/\* ==== (.*?) @ ([0-9a-f]+) ==== \*/',p)
    if m and (m.group(2) in sys.argv[2:] or any(a in m.group(1) for a in sys.argv[2:] if not re.fullmatch(r'[0-9a-f]{8}',a))):
        print(p)
