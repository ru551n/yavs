-- 2019 shared variable of an array of a protected type (LCS-2016-014).
entity top is end;
architecture a of top is
  type pt is protected procedure inc; impure function get return integer; end protected;
  type pt is protected body
    variable n : integer := 0;
    procedure inc is begin n := n + 1; end;
    impure function get return integer is begin return n; end;
  end protected body;
  type pa is array (0 to 1) of pt;
  shared variable sv : pa;
begin
  p1 : process begin sv(1).inc; wait; end process;
  p2 : process begin sv(1).inc; wait; end process;
  p3 : process begin
    wait for 1 ns;
    report "RES " & integer'image(sv(0).get) & integer'image(sv(1).get);
    wait;
  end process;
end;
