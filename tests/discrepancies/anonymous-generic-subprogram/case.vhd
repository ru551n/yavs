-- A 2019 subprogram with both a generic list and a parameter of an anonymous type (`type is ...`).
package p is
  function pick generic (type t) parameter (x : type is private; y : t) return t;
end package;
package body p is
  function pick generic (type t) parameter (x : type is private; y : t) return t is begin return y; end function;
end package body;
entity top is end;
architecture a of top is
  function pi is new work.p.pick generic map (t => integer);
begin
  process begin
    report "RES " & integer'image(pi(true, 5)) & " " & integer'image(pi(1.5, 6));
    wait;
  end process;
end architecture;
