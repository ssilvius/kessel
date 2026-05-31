import re, sys

MOD='kessel/src/db/mod.rs'
SCHEMA='kessel/src/db/schema.rs'

def rd(p): return open(p).read().split('\n')
def wr(p,l): open(p,'w').write('\n'.join(l))

def backtrack_docs(lines, i):
    while i>0 and (lines[i-1].lstrip().startswith('///') or lines[i-1].lstrip().startswith('//')
                   or lines[i-1].lstrip().startswith('#[')):
        i-=1
    return i

def method_span(lines, name):
    pat=re.compile(rf'^    (pub(\(crate\))? )?fn {re.escape(name)}\b')
    for i,l in enumerate(lines):
        if pat.match(l):
            s=backtrack_docs(lines,i)
            for j in range(i,len(lines)):
                if lines[j]=='    }':
                    return (s,j)
    return None

def struct_span(lines, name):
    import re
    pat=re.compile(rf'^(pub(\(crate\))? )?struct {re.escape(name)}\b')
    for i,l in enumerate(lines):
        if pat.match(l):
            s=backtrack_docs(lines,i)
            for j in range(i,len(lines)):
                if lines[j]=='}':
                    return (s,j)
    return None

def impl_span(lines, name):
    import re
    pat=re.compile(rf'^impl {re.escape(name)}\b')
    for i,l in enumerate(lines):
        if pat.match(l):
            for j in range(i,len(lines)):
                if lines[j]=='}':
                    return (i,j)
    return None

def freefn_span(lines, name):
    pat=re.compile(rf'^(pub(\(crate\))? )?fn {re.escape(name)}\b')
    for i,l in enumerate(lines):
        if pat.match(l):
            s=backtrack_docs(lines,i)
            for j in range(i,len(lines)):
                if lines[j]=='}':
                    return (s,j)
    return None

def test_span(lines, name):
    pat=re.compile(rf'^    fn {re.escape(name)}\b')
    for i,l in enumerate(lines):
        if pat.match(l):
            s=backtrack_docs(lines,i)  # grabs #[test]/#[should_panic]
            for j in range(i,len(lines)):
                if lines[j]=='    }':
                    return (s,j)
    return None

def ddl_spans(lines, tables):
    spans=[]
    for t in tables:
        # CREATE TABLE block (+ preceding comment lines)
        ct=re.compile(rf'^\s*CREATE TABLE IF NOT EXISTS {re.escape(t)}\b')
        for i,l in enumerate(lines):
            if ct.match(l):
                s=i
                while s>0 and lines[s-1].strip().startswith('--'):
                    s-=1
                for j in range(i,len(lines)):
                    if lines[j].strip()==');':
                        spans.append((s,j)); break
        # CREATE INDEX ... ON t(  (single or two-line)
        ci=re.compile(rf'^\s*CREATE INDEX IF NOT EXISTS \S+\s+ON {re.escape(t)}\b|^\s*CREATE INDEX IF NOT EXISTS \S+$')
        i=0
        while i<len(lines):
            l=lines[i]
            m=re.match(r'^\s*CREATE INDEX IF NOT EXISTS (\S+)', l)
            if m:
                # find the ON target (this line or next)
                blob=l
                end=i
                if 'ON ' not in l and i+1<len(lines):
                    blob=l+' '+lines[i+1].strip(); end=i+1
                mo=re.search(r'ON (\w+)\b', blob)
                if mo and mo.group(1)==t:
                    spans.append((i,end))
            i+=1
    return spans

def remove_spans(lines, spans):
    # spans: list of (start,end) inclusive; remove highest first
    for s,e in sorted(set(spans), key=lambda x:-x[0]):
        del lines[s:e+1]

def extract_text(lines, spans):
    # return list of text blocks, in original order
    return ['\n'.join(lines[s:e+1]) for s,e in sorted(set(spans))]

if __name__=='__main__':
    print("helper module; import and drive per domain")

def mkpub_free(b):
    import re
    lines=b.split('\n')
    for i,l in enumerate(lines):
        if re.match(r'^(pub )?fn ',l):
            lines[i]=re.sub(r'^(pub )?fn ','pub(crate) fn ',l); return '\n'.join(lines)
    return b

def drive(domain, methods, freefns, tests, tables, doc, extra_top='', structs=None, impls=None):
    import re
    mod=rd(MOD); sch=rd(SCHEMA)
    mspans=[method_span(mod,n) for n in methods]
    assert all(mspans), "missing methods: "+str([methods[i] for i,s in enumerate(mspans) if not s])
    m_text=extract_text(mod,mspans)
    fspans=[freefn_span(mod,n) for n in freefns]
    assert all(fspans), "missing freefns: "+str([freefns[i] for i,s in enumerate(fspans) if not s])
    f_text=[mkpub_free(b) for b in extract_text(mod,fspans)]
    structs=structs or []
    sspans=[struct_span(mod,n) for n in structs]
    assert all(sspans), "missing structs: "+str([structs[i] for i,s in enumerate(sspans) if not s])
    s_text=[mkpub_free(b) for b in extract_text(mod,sspans)]
    impls=impls or []
    ispans=[impl_span(mod,n) for n in impls]
    assert all(ispans), "missing impls: "+str([impls[i] for i,s in enumerate(ispans) if not s])
    i_text=extract_text(mod,ispans)
    tspans=[test_span(mod,n) for n in tests]
    assert all(tspans), "missing tests: "+str([tests[i] for i,s in enumerate(tspans) if not s])
    t_text=extract_text(mod,tspans)
    dspans=ddl_spans(sch,tables); d_text=extract_text(sch,dspans)

    remove_spans(mod, mspans+fspans+tspans+sspans+ispans)
    remove_spans(sch, dspans)

    parts=[f'//! {doc}','','use super::*;']
    if extra_top: parts.append(extra_top)
    parts+=['', 'impl Database {', '\n\n'.join(m_text), '}','']
    if s_text: parts+=['\n\n'.join(s_text),'']
    if i_text: parts+=['\n\n'.join(i_text),'']
    if f_text: parts+=['\n\n'.join(f_text),'']
    parts+=[f'/// Create the {domain} tables (idempotent).',
            'pub(crate) fn create_tables(tx: &Transaction) -> Result<()> {',
            '    tx.execute_batch(','        r#"','\n'.join(d_text),'        "#,','    )?;','    Ok(())','}','']
    if t_text:
        parts+=['#[cfg(test)]','mod tests {','    use super::*;',
                '    use crate::db::testutil::*;','\n'.join(t_text),'}','']
    open(f'kessel/src/db/{domain}.rs','w').write('\n'.join(parts))

    # wire mod.rs: add `mod <domain>;` after `mod quest;` (the last domain decl) and init call
    out=[]; added_mod=False
    for l in mod:
        out.append(l)
    # insert mod decl after `mod schema;`
    for i,l in enumerate(out):
        if l=='mod schema;':
            out.insert(i+1, f'mod {domain};'); break
    for i,l in enumerate(out):
        if 'schema::create_tables(&tx)?;' in l:
            out.insert(i+1, f'            {domain}::create_tables(&tx)?;'); break
    wr(MOD,out); wr(SCHEMA,sch)
    print(f"{domain}: {len(m_text)} methods, {len(f_text)} freefns, {len(t_text)} tests, {len(d_text)} ddl")
