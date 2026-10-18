-- yavs-test: std=2008
-- expect: E1002 4:10 unit `nopkg` not found in library `work`
-- LRM 13.4
use work.nopkg.all;
package q is
end package;
