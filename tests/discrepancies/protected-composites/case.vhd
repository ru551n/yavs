-- 2019 arrays, records and access types of protected types (LCS-2016-014, 014a): each element
-- is its own initialised object; method calls through indexing, selection and `.all`.
entity top is end;
architecture a of top is
  type pt is protected procedure inc; impure function get return integer; end protected;
  type pt is protected body
    variable n : integer := 0;
    procedure inc is begin n := n + 1; end;
    impure function get return integer is begin return n; end;
  end protected body;
  type pa is array (0 to 1) of pt;
  type pap is access pa;
  type rec is record a : pt; c : pa; end record;
begin
  process
    variable v : pa;
    variable p : pap;
    variable r : rec;
  begin
    v(1).inc;
    p := new pa; p(1).inc; p.all(1).inc;
    r.c(0).inc;
    report "RES " & integer'image(v(0).get) & integer'image(v(1).get) & integer'image(p(1).get)
      & integer'image(r.a.get) & integer'image(r.c(0).get);
    deallocate(p);
    wait;
  end process;
end;
