-- LRM 6.5.7.2, 11.2: a generic subprogram of a block as the actual of an entity's generic
-- subprogram.
entity cell is
  generic (type t; init : t; function f (x : t) return t);
  port (q : out t);
end entity;
architecture a of cell is
begin
  q <= f(f(init));
end architecture;
entity top is end entity;
architecture a of top is
  function inc (x : integer) return integer is begin return x + 1; end function;
  signal q : integer;
begin
  b : block
    generic (type t; function g (x : t) return t);
    generic map (t => integer, g => inc);
  begin
    u : entity work.cell generic map (t => t, init => 1, f => g) port map (q);
  end block;
  process begin wait for 1 ns; report "RES block ok"; wait; end process;
end architecture;
