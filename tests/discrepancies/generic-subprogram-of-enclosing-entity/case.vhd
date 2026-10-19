-- LRM 6.5.7.2, 7.3.3: a generic subprogram of an enclosing entity passed down as the actual of
-- a nested instance's generic subprogram.
entity cell is
  generic (type t; init : t; function f (x : t) return t);
  port (q : out t);
end entity;
architecture a of cell is
begin
  q <= f(f(init));
end architecture;
entity wrap is
  generic (type t; init : t; function g (x : t) return t);
end entity;
architecture a of wrap is
  signal q : t;
begin
  u : entity work.cell generic map (t => t, init => init, f => g) port map (q);
  process begin wait for 1 ns; report "RES wrap ok"; wait; end process;
end architecture;
entity top is end entity;
architecture a of top is
  function inc (x : integer) return integer is begin return x + 1; end function;
begin
  w : entity work.wrap generic map (t => integer, init => 1, g => inc);
end architecture;
