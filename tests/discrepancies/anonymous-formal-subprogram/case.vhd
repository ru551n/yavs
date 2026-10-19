-- A 2019 formal subprogram whose parameter has an anonymous type (`type is ...`): the type is the one of the actual's profile.
package p is
  generic (function f (x : type is private) return integer);
  function k return integer;
end package;
package body p is
  function k return integer is begin return 1; end function;
end package body;
entity top is end;
architecture a of top is
  function seven (b : boolean) return integer is begin return 7; end function;
  package pi is new work.p generic map (f => seven);
begin
  process begin
    report "RES " & integer'image(pi.k);
    wait;
  end process;
end architecture;
