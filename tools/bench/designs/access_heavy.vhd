-- Access types as queues and message buffers use them: records in a linked list, strings
-- allocated and released, an array of access values.
entity bench_access_heavy is
  generic (N : natural := 100000);
end entity;

architecture sim of bench_access_heavy is
  type node_t;
  type node_ptr is access node_t;
  type node_t is record
    value : integer;
    name  : string(1 to 8);
    next_n : node_ptr;
  end record;
  type str_ptr is access string;
  type str_ptr_arr is array (0 to 63) of str_ptr;
begin
  process
    variable head, tail, nd : node_ptr;
    variable cnt : natural := 0;
    variable sum : natural := 0;
    variable s   : str_ptr;
    variable pool : str_ptr_arr;
  begin
    for i in 1 to N loop
      nd := new node_t'(value => i, name => "item0001", next_n => null);
      if head = null then
        head := nd;
      else
        tail.next_n := nd;
      end if;
      tail := nd;
      cnt := cnt + 1;
      if cnt > 32 then
        nd := head;
        head := head.next_n;
        sum := (sum + nd.value) mod 1000003;
        deallocate(nd);
        cnt := cnt - 1;
      end if;
      s := new string'("buffer " & integer'image(i));
      sum := (sum + s'length) mod 1000003;
      if pool(i mod 64) /= null then
        deallocate(pool(i mod 64));
      end if;
      pool(i mod 64) := s;
    end loop;
    report "access_heavy sum=" & integer'image(sum);
    wait;
  end process;
end architecture;
