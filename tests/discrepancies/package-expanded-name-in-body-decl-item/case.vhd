-- LRM 8.3, 12.1: inside the body of package `p`, `p.s` names `s` of its declaration (#854)
package p is
  constant s : integer := 3;
  function get return integer;
end package;
package body p is
  function get return integer is begin return p.s; end function;
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
