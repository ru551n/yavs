-- A 2019 generic constant and a port with an anonymous type (`type is ...`): the type is the actual's.
package g is
  generic (c : type is range <>; k : c'subtype);
  function sum return c'subtype;
end package;
package body g is
  function sum return c'subtype is begin return c + k; end function;
end package body;
package gi is new work.g generic map (c => 3, k => 4);
entity e is
  generic (n : type is range <>; m : n'subtype);
  port (r : out n'subtype);
end entity;
architecture a of e is begin r <= n + m; end architecture;
entity top is end;
architecture a of top is
  signal o : integer;
begin
  u : entity work.e generic map (n => 10, m => 20) port map (r => o);
  process begin
    wait for 1 ns;
    report "RES " & integer'image(work.gi.sum) & " " & integer'image(o);
    wait;
  end process;
end architecture;
