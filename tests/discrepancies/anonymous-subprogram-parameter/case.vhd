-- A 2019 subprogram parameter with an anonymous type (`type is ...`): each call binds it to its actual's type.
package p is
  function add (a : type is range <>; b : a'subtype) return a'subtype;
end package;
package body p is
  function add (a : type is range <>; b : a'subtype) return a'subtype is begin return a + b; end function;
end package body;
entity top is end;
architecture a of top is
begin
  process begin
    report "RES " & integer'image(work.p.add(1, 2)) & " " & integer'image(work.p.add(natural'(5), 6));
    wait;
  end process;
end architecture;
