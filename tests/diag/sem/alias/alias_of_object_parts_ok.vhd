-- yavs-test: std=2008
-- expect: none
-- LRM 6.6.2 (VUnit run_api: aliases of slices; sim1 and sim2 accept)
package p is
  type rec is record a : bit_vector(7 downto 0); b : integer; end record;
  constant first : natural := 2;
  signal runner : bit_vector(0 to 15);
  alias runner_phase is runner(first to first + 3);
  alias runner_bit : bit is runner(0);
  signal r : rec;
  alias ra is r.a;
  alias ra_hi : bit_vector(3 downto 0) is r.a(7 downto 4);
  constant c : rec := (a => x"00", b => 1);
  alias cb is c.b;
end package;
