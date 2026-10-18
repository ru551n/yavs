-- -----------------------------------------------------------------
-- 
-- Copyright 2019 IEEE P1076 WG Authors
-- 
-- See the LICENSE file distributed with this work for copyright and
-- licensing information and the AUTHORS file.
-- 
-- This file to you under the Apache License, Version 2.0 (the "License").
-- You may obtain a copy of the License at
-- 
--     http://www.apache.org/licenses/LICENSE-2.0
-- 
-- Unless required by applicable law or agreed to in writing, software
-- distributed under the License is distributed on an "AS IS" BASIS,
-- WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or
-- implied.  See the License for the specific language governing
-- permissions and limitations under the License.
--
-- yavs: derived from the IEEE VASG 1076-2019 package source by
-- tools/derive_libs.py (see lib/SOURCES.md for the exact edits).
--
--   Title     :  ENV package (1076-2008 content)
--

package ENV is
  procedure STOP (STATUS: INTEGER);
  procedure STOP;
  procedure FINISH (STATUS: INTEGER);
  procedure FINISH;
  function RESOLUTION_LIMIT return DELAY_LENGTH;
end package ENV;
