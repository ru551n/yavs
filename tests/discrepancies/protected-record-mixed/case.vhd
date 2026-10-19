-- 2019 record with a protected element and an element of another type (LCS-2016-014).
entity top is end;
architecture a of top is
  type pt is protected procedure inc; impure function get return integer; end protected;
  type pt is protected body
    variable n : integer := 0;
    procedure inc is begin n := n + 1; end;
    impure function get return integer is begin return n; end;
  end protected body;
  type rec is record a : pt; b : integer; end record;
begin
  process
    variable r : rec;
  begin
    r.b := 4; r.a.inc;
    report "RES " & integer'image(r.b) & integer'image(r.a.get);
    wait;
  end process;
end;
