-- LRM 8.3, 12.1: inside the body of package `p`, `p.b` names `b` of the body (#854)
package p is
  function get return integer;
end package;
package body p is
  constant b : integer := 4;
  function get return integer is begin return p.b; end function;
end package body;
entity top is end entity;
architecture a of top is
begin
  process
  begin
    report "RES get=" & integer'image(work.p.get);
    wait;
  end process;
end architecture;
