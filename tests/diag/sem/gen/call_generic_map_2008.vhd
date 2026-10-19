-- yavs-test: std=2008
-- expect: E0004 10:32 a generic map in a call requires VHDL-2019
-- LCS-2016-049: a generic map in a call needs VHDL-2019
entity e is end entity;
architecture a of e is
  function ident generic (type t) parameter (x : t) return t is begin return x; end function;
begin
  process
  begin
    report integer'image(ident generic map (t => integer) (4));
    wait;
  end process;
end architecture;
