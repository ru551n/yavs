-- difftest: top=shared_var_regions std=2008
-- A shared variable declared in a block or generate statement is one object per elaboration
-- of that declarative part (LRM 14.4, 6.4.2.4): processes of nested blocks and generates use
-- the object of the enclosing region, each for generate iteration has its own (#249).
package shared_var_regions_p is
  type counter_t is protected
    procedure add (n : integer);
    impure function get return integer;
  end protected;
end package;
package body shared_var_regions_p is
  type counter_t is protected body
    variable v : integer := 0;
    procedure add (n : integer) is begin v := v + n; end procedure;
    impure function get return integer is begin return v; end function;
  end protected body;
end package body;

use work.shared_var_regions_p.all;
entity shared_var_regions is
end entity;

architecture a of shared_var_regions is
begin
  b : block
    shared variable sv : counter_t;
  begin
    process begin sv.add(1); wait; end process;
    inner : block
    begin
      process begin sv.add(2); wait; end process;
      deep : for k in 1 to 2 generate
        process begin sv.add(10 * k); wait; end process;
      end generate;
    end block;
    process
    begin
      wait for 1 ns;
      report "b: " & integer'image(sv.get);
      assert sv.get = 33 severity failure;
      wait;
    end process;
  end block;

  g : if true generate
    shared variable sv : counter_t;
  begin
    process
    begin
      sv.add(1);
      wait for 2 ns;
      report "g: " & integer'image(sv.get);
      assert sv.get = 3 severity failure;
      wait;
    end process;
    w : for i in 1 to 2 generate
      process begin sv.add(1); wait; end process;
    end generate;
  end generate;

  f : for i in 1 to 3 generate
    shared variable sv : counter_t;
  begin
    process begin sv.add(i); wait; end process;
    nb : block
    begin
      process begin sv.add(100); wait; end process;
      ig : if i > 1 generate
        process begin sv.add(1000); wait; end process;
      end generate;
    end block;
    process
    begin
      wait for (2 + i) * 1 ns;
      report "f(" & integer'image(i) & "): " & integer'image(sv.get);
      if i > 1 then
        assert sv.get = i + 1100 severity failure;
      else
        assert sv.get = i + 100 severity failure;
      end if;
      wait;
    end process;
  end generate;
end architecture;
