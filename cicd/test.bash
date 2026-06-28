#!/bin/bash

## Active shellchecks
# shellcheck disable=1090
# shellcheck disable=1091
# shellcheck disable=2001   ## Complaining about use of sed istead of bash search & replace.
# shellcheck disable=2002   ## Useless use of cat. This works well though and I don't want to break it for the sake of syntax purity.
# shellcheck disable=2004   ## Inappropriate complaining of "$/${} is unnecessary on arithmetic variables."
# shellcheck disable=2119   ## Disable confusing and inapplicable warning about function's $1 meaning script's $1.
# shellcheck disable=2120   ## OK with declaring variables that accept arguments, without calling with arguments (this is 'overloading').
# shellcheck disable=2143   ## Used grep -q instead of echo | grep
# shellcheck disable=2154
# shellcheck disable=2155   ## Disable check to 'Declare and assign separately to avoid masking return values'.
# shellcheck disable=2162
# shellcheck disable=2181
# shellcheck disable=2207
# shellcheck disable=2317   ## Can't reach

## Inactive shellchecks
## shellcheck disable=2034  ## Unused variables.

##	Purpose:
##		- CI/CD-friendly test harness that passes or fails.
##		- Tests random output and round-trips through v2 to make sure the initial output was correct (at least if v2 is also correct).
##		- This is NOT part of cicd script, as it's not a requirement to have v2 installed.
##	History: At bottom of this file. (Note: History for this is maintained outside of [or in addition to] git project.)

##	Copyright © 2026 Jim Collier (ID: 1cv◂‡Vᛦ)
##	Licensed under The MIT License (MIT). Full text at:
##		https://mit-license.org/
##	SPDX-License-Identifier: MIT


## Global settings
set -e
if [[ -z "${doLongTest+x}" ]]; then
	declare     doLongTest=0 ; [[ "${CICDTEST_DO_LONGTEST}" == "1" ]] && doLongTest=1
	declare -ri doTestAllBaseCombos=$(( $(od -An -N1 -tu1 /dev/urandom) % 10 == 0 ? 1 : 0 ))  ## 10% odds of running all base combo tests (long)
	declare -ri doBackwardsCompatTests=1
fi


fMain_Test(){

	## Settings (paths are canonicalized and validated relative to this script)
	local     exeV2="../source/bin/convert-base-v2"
	local     exeV1b="utility/convert-base-v1b"
	local     baseDefs="include/base-definitions.bash"

	## Environment overrides
	local     LANG="C.UTF-8"  ## Splitting won't work correctly without this

	## Resolve paths
	fResolvePath  exeV2      "${exeV2}"         ; readonly exeV2
	fResolvePath  exeV1b     "${exeV1b}"     0  ; readonly exeV1b  ## Doesn't need to exist, can run tests without. This is to verify backwards-compatibility.
	fResolvePath  baseDefs   "${baseDefs}"      ; readonly baseDefs

	## Compare to exeV2?
	local -i doComareWith_v1=0
	{ ((doBackwardsCompatTests))  &&  [[ -x "${exeV1b}" ]]; }  &&  doComareWith_v1=1
	readonly doComareWith_v1

	## Load base definitions arrays
	fEcho_Clean
	source "${baseDefs}"
	fEcho_Clean_Force

	## Variables
	local inputVal=""  expectVal=""  gotVal=""
	local -i loopCount=0

	####
	#### Will they even load at all

	fEcho_Clean
	fEcho_Clean "Exe source ...: ${exeV2}"
	fEcho_Clean "Version ......: $("${exeV2}" --version)"
	fEcho_Clean_Force
	sleep 2
	if ((doComareWith_v1)); then
		fEcho_Clean "v1b source ...: ${exeV1b}"
		fEcho_Clean "Version ......: $("${exeV1b}" --version)"
		fEcho_Clean_Force
		sleep 2
	fi


	####
	#### Test flags (make sure -e is enabled)

	fEcho; fEcho ">>> TESTSECTION: Flags"; fEcho

	fEcho; fEcho "Test --help"
	"${exeV1b}" --help

	fEcho "Test --about"
	"${exeV1b}" --about

	fEcho "Test --version (again)"
	"${exeV1b}" --version
	fEcho_Clean_Force


	####
	#### Force loading of any bases; don't care about output value

	fEcho; fEcho "Testing internal load of bases ..."
	"${exeV1b}" --to 16 --from 10 255
	sleep 1
	"${exeV1b}" --to '288jc1' --from 10 1024
	sleep 1

	####
	#### Specific testing loops

	## Aliases
	fEcho; fEcho ">>> TESTSECTION: Base name aliases"; fEcho
	inputVal="987654321000055555555550000123456789" #....................................: The value is less important than just the aliases. But also, a large value shouldn't fail either.
	fTestAllAliases  "base10"  "${inputVal}" #...........................................: All should pass
	fRunTest  'error'  "${expectVal}"  "'${exeV2}'  '${inputVal}'  bogusBaseName" #......: This one should fail

	if ((doTestAllBaseCombos)); then
		## Self-test every base against every other base (with fixed-lenght but randomized input).
		fEcho; fEcho ">>> TESTSECTION: Test all bases against each other"; fEcho
		fTest_AllBasesAgainstEachOther
	fi


	####
	#### Generate loop counter for next sections. Random # between 10 and 150. (So most of the time it's on the fast side.)
	if ((doLongTest)); then
		loopCount=5000
	else
		tmpRandomBaseIdx=$((10 + $(od -An -N2 -tu2 /dev/urandom) % (150 +1) ))
	fi
	loopCount=80
	((doLongTest))  &&


	####
	#### Looped random fuzz-testing

	## Test **AGAINST SELF**
	fEcho; fEcho ">>> TESTSECTION: Fuzz-testing against self"; fEcho
	fFuzzTest_Self

	#### Test **AGAINST v1b** (all the bases, v2 is input, v1b is output)
	fEcho; fEcho ">>> TESTSECTION: Fuzz-testing against v1b (all bases)"; fEcho
	((doComareWith_v1))  &&  fFuzzTest_Base10_To_BaseX_AndBack_via_v1b


	####
	#### Test val to use for next sections
	inputVal="01234567899999999999999990123456789999999999999999123456789999999990000000000000000000000000000000000000000000000099999999999999999999999999999999999999876543210"


	####
	#### By-hand one-way tests, expect equal
	fEcho; fEcho ">>> TESTSECTION: By-hand one-way tests, expect equal"; fEcho

	## 128v1compat
	#expectVal="$(convert-base-v1  "${inputVal}"  128j1)"  #; echo "${expectVal}"
	expectVal="FrĜЋŝĴR2§⁑⍤🝅⌲μr1ϟỹẼ⌲M§ỹλ🜥ψ🝅ᛘêᚼ75ĜᛝmÑ🜥Ĝλŝ▵ϠĜRλΞãᛎ8hÊᛯĝĵΩJĜ▿ĤxŴĵ£Cᛏẅ8ÂψvÉÉδPĝŷ"
	fRunTest  '=='  "${expectVal}"  "'${exeV2}'  ${inputVal}  128v1compat"

	## 128j1
	#expectVal="$(convert-base-v2  "${inputVal}"  128jc1)"  # ; echo "${expectVal}"
	expectVal="BUΩᛨ¢ΞI2🝅x◂p‡aU1ᛦ⍋¿‡F🝅⍋ZnᛘpdЖl75ΩfRɤnΩZ¢qᛯΩIZᛏ⍤b8P≠eЯфμEΩsƱXϠф🜥AcÎ8∞ᛘVŴŴᛝGЯ¥"
	fRunTest  '=='  "${expectVal}"  "'${exeV2}'  ${inputVal}  128jc1"


	####
	#### By-hand one-way tests, expect NOT equal
	fEcho; fEcho ">>> TESTSECTION: By-hand one-way tests, expect NOT equal"; fEcho

	## 128j1 != 128v1compat
	#expectVal="$(convert-base-v1  "${inputVal}"  128j1)"  #; echo "${expectVal}"
	expectVal="FrĜЋŝĴR2§⁑⍤🝅⌲μr1ϟỹẼ⌲M§ỹλ🜥ψ🝅ᛘêᚼ75ĜᛝmÑ🜥Ĝλŝ▵ϠĜRλΞãᛎ8hÊᛯĝĵΩJĜ▿ĤxŴĵ£Cᛏẅ8ÂψvÉÉδPĝŷ"
	fRunTest  '!='  "${expectVal}"  "'${exeV2}'  ${inputVal}  128jc1"


	####
	#### By-hand one-way tests, expect ERROR
	fEcho; fEcho ">>> TESTSECTION: By-hand one-way tests, expect ERROR"; fEcho

	## Removed base 16 as input, should error.
	expectVal=""
	fRunTest  'error'  "[anything or nothing]"  "'${exeV2}'  --from 201  'ABCXYZ'  10"


	####
	#### By-hand round-trips self-tests, expect equal.
	fEcho; fEcho ">>> TESTSECTION: By-hand round-trip tests, expect equal"; fEcho

	expectVal="1234567899999999999999990123456789999999999999999123456789999999990000000000000000000000000000000000000000000000099999999999999999999999999999999999999876543210"
	fRunChained_TestLast  '=='  "${expectVal}"  "'${exeV2}'  --from 10  --to 16  ${inputVal}; '${exeV2}'  --from 16  --to 10  %CMD1_OUTPUT%"

:;}


fTestAllAliases(){
	local -r inputBase="${1:-}"  ; shift || true
	local -r inputVal="${1:-}"   ; shift || true
	for nextBase in "${baseAliasesArr[@]}"; do
		fRunTest  'no_error'  "[anything or nothing]"  "'${exeV2}'  --from ${inputBase}  ${inputVal}  ${nextBase}"
	done;:
}


fTest_AllBasesAgainstEachOther(){

	## Settings
	local -ri fixed_InputLen=64
	local -ri count_TotalDefinedBases_Input=${#bases_Input_IdxToKey[@]}
	local -ri count_TotalDefinedBases_Output=${#bases_Output_IdxToKey[@]}  ## Will hopefully be the same as input, but not necessarily forever and always in the future.

	## Loop variables
	local  -i tmpRandomBaseIdx=-1
	local     inputStr=""
	local     inputBaseName=""
	local     inputBaseSymbols=""
	local     intermediateBaseName=""
	local     intermediateVal=""
	local     exeV2name=""
	local     exeV2args=""

	for ((i=0; i<count_TotalDefinedBases_Input; i++)); do
		## Since in this case input and output bases are the same, we don't have to loop
		##   over the entire output base array each time for each input, because that
		##   would result in a lot of redundancy.
		## But since this TECHNICALLY isn't a self-compare, and the bases COULD someday be
		##   different, let's go ahead and compare every i to every j.
		for ((j=0; j<count_TotalDefinedBases_Output; j++)); do

			## Get input base [i] and its list of symbols
			inputBaseName="${bases_Input_IdxToKey[i]}"
			inputBaseSymbols="${bases_Input_KeyToVal["${inputBaseName}"]}"

			## Get a random input of random in-base symbols, of fixed length
			fScrambleString  inputStr  "${inputBaseSymbols}"   $fixed_InputLen

			## To avoid falsely triggering an error:
			## Strip off leading symbols representing '0' from input, which will be gone from the output during conversion.
			expectVal="${inputStr}"
			until [[ "${expectVal:0:1}" !=  "${inputBaseSymbols:0:1}" ]]; do expectVal="${expectVal:1}"; done;:
			[[ -z "${expectVal}" ]]  &&  continue  ## If it's empty now, just skip to next test.

			## Get intermediate base [j] and its list of symbols
			intermediateBaseName="${bases_Output_IdxToKey[j]}"

			## Format and prepare the first command for display, to be shown in output (via variable "hook"); and run it
			exeV2name=""  exeV2args=""
			fGetIsolatedExeName  exeV2name  exeV2args  "'${exeV2}'  --from '${inputBaseName}'  --to '${intermediateBaseName}'  --  '${expectVal}'"
			__fRunTest_EchoHook1="Cmd 1 ..........: '${exeV2name}'${exeV2args}"
			intermediateVal="$("${exeV2}"  --from "${inputBaseName}"  --to "${intermediateBaseName}"  --  "${expectVal}")"

			#DEBUG
			#sleep 2
			#echo
			#echo "inputBaseName ...............: ${inputBaseName}"
			#echo "inputBaseSymbols ............: ${inputBaseSymbols}"
			#echo "inputStr ....................: ${inputStr}"
			#echo "expectVal ...................: ${expectVal}"
			#echo "intermediateBaseName ........: ${intermediateBaseName}"
			#echo "intermediateVal .............: ${intermediateVal}"
			#echo

			## Run the second command with the previous command's output as this command's input.
			## This command's output should be the same as the previous command's input.
			fRunTest  '=='  "${expectVal}"  "'${exeV2}'  --from '${intermediateBaseName}'  --to '${inputBaseName}'  --  '${intermediateVal}'"

		:; done;:
	:; done;:

:;}


fFuzzTest_Self(){

	## Settings
	local -ri maxTestInputChars=1024
	local -ri count_TotalDefinedBases_Input=${#bases_Input_IdxToKey[@]}
	local -ri count_TotalDefinedBases_Output=${#bases_Output_IdxToKey[@]}  ## Will hopefully be the same as input, but not necessarily forever and always in the future.

	## Loop variables
	local -i  tmpRandomBaseIdx=-1
	local -i  random_InputLen=0
	local     inputStr=""
	local     inputBaseName=""
	local     inputBaseSymbols=""
	local     intermediateBaseName=""
	local     intermediateVal=""
	local     exeV2name=""
	local     exeV2args=""

	for ((i=1; i<=loopCount; i++)); do

		## Get a random input base and its list of symbols
		tmpRandomBaseIdx=$((0 + $(od -An -N1 -tu2 /dev/urandom) % (count_TotalDefinedBases_Input - 1) ))
		inputBaseName="${bases_Input_IdxToKey[tmpRandomBaseIdx]}"
		inputBaseSymbols="${bases_Input_KeyToVal["${inputBaseName}"]}"

		## Get a random input of random in-base symbols, of random length
		random_InputLen=$((1 + $(od -An -N2 -tu2 /dev/urandom) % maxTestInputChars))
		[[ -z "${inputBaseSymbols}" ]] && { echo -e "\nError in ${meName_t4rgd}.${FUNCNAME[0]}(): \$inputBaseSymbols == '', aborting.\n" ; exit 1; }
		((random_InputLen <=0))        && { echo -e "\nError in ${meName_t4rgd}.${FUNCNAME[0]}(): \$random_InputLen == 0, aborting.\n"   ; exit 1; }
		fScrambleString  inputStr  "${inputBaseSymbols}"   $random_InputLen

		## To avoid falsely triggering an error:
		## Strip off leading symbols representing '0' from input, which will be gone from the output during conversion.
		expectVal="${inputStr}"
		until [[ "${expectVal:0:1}" !=  "${inputBaseSymbols:0:1}" ]]; do expectVal="${expectVal:1}"; done;:
		[[ -z "${expectVal}" ]]  &&  continue  ## If it's empty now, just skip to next test.

		## Pick a random intermediate output base
		tmpRandomBaseIdx=$((0 + $(od -An -N1 -tu2 /dev/urandom) % (count_TotalDefinedBases_Output - 1) ))
		intermediateBaseName="${bases_Output_IdxToKey[tmpRandomBaseIdx]}"

		## Format and prepare the first command for display, to be shown in output (via variable "hook"); and run it
		exeV2name=""  exeV2args=""
		fGetIsolatedExeName  exeV2name  exeV2args  "'${exeV2}'  --from '${inputBaseName}'  --to '${intermediateBaseName}'  --  '${expectVal}'"
		__fRunTest_EchoHook1="Cmd 1 ..........: '${exeV2name}'${exeV2args}"
		intermediateVal="$("${exeV2}"  --from "${inputBaseName}"  --to "${intermediateBaseName}"  --  "${expectVal}")"

		##DEBUG
		#sleep 2
		#echo
		#echo "inputBaseName ...............: ${inputBaseName}"
		#echo "inputBaseSymbols ............: ${inputBaseSymbols}"
		#echo "random_InputLen .............: ${random_InputLen}"
		#echo "inputStr ....................: ${inputStr}"
		#echo "expectVal ...................: ${expectVal}"
		#echo "intermediateBaseName ........: ${intermediateBaseName}"
		#echo "intermediateVal .............: ${intermediateVal}"
		#echo

		## Run the second command with the previous command's output as this command's input.
		## This command's output should be the same as the previous command's input.
		fRunTest  '=='  "${expectVal}"  "'${exeV2}'  --from '${intermediateBaseName}'  --to '${inputBaseName}'  --  '${intermediateVal}'"

	done;:

}

fFuzzTest_Base10_To_BaseX_AndBack_via_v1b(){

	## Settings
	local -ri maxTestInputChars=256
	local -ri count_TotalDefinedBases_Output=${#bases_Output_IdxToKey[@]}

	## Loop variables
	local -i  random_InputLen=0
	local     inputStr=""
	local -i  tmpRandomBaseIdx=-1
	local     intermediateBaseName=""
	local     intermediateVal=""
	local     exeV1bname=""
	local     exeV1bargs=""

	for ((i=1; i<=loopCount; i++)); do

		## Generate a random base 10 number for first input
		random_InputLen=$((1 + $(od -An -N2 -tu2 /dev/urandom) % maxTestInputChars))
		fScrambleString  inputStr  "0123456789"   $random_InputLen

		## To avoid falsely triggering an error:
		## Strip off leading symbols representing '0' from input, which will be gone from the output during conversion.
		shopt -s extglob
		expectVal="${inputStr#"${inputStr%%[!0]*}"}"
		[[ -z "${expectVal}" ]]  &&  continue

		## Pick a random intermediate v1b output -> v2 input base
		tmpRandomBaseIdx=$((0 + $(od -An -N1 -tu2 /dev/urandom) % (count_TotalDefinedBases_Output - 1) ))
		intermediateBaseName="${bases_Output_IdxToKey[tmpRandomBaseIdx]:-}"

		## Format and prepare the first command for display, to be shown in output (via variable "hook"); and run it
		exeV1bname=""  exeV1bargs=""
		fGetIsolatedExeName  exeV1bname  exeV1bargs  "'${exeV1b}'  --ibase 10  '${expectVal}'  '${intermediateBaseName}'"
		__fRunTest_EchoHook1="Cmd 1 ..........: '${exeV1bname}'${exeV1bargs}"
		intermediateVal="$("${exeV1b}"  --ibase 10  "${expectVal}"  "${intermediateBaseName}")"

		##DEBUG
		#echo
		#echo "random_InputLen .............: ${random_InputLen}"
		#echo "inputStr ....................: ${inputStr}"
		#echo "expectVal ...................: ${expectVal}"
		#echo "tmpRandomBaseIdx ...: ${tmpRandomBaseIdx}"
		#echo "intermediateBaseName ........: ${intermediateBaseName}"
		#echo "intermediateVal .............: ${intermediateVal}"
		#sleep 5

		## Run the second command with the previous command's output as this command's input.
		## This command's output should be the same as the previous command's input.
		fRunTest  '=='  "${expectVal}"  "'${exeV2}'  --from '${intermediateBaseName}'  --to 10  --  '${intermediateVal}'"

	done;:

}


#••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••
## Generic function prototypes for reference and linting correctness. Overridden with real function when generic script is sourced at the bottom of this script.
#••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••
# shellcheck disable=SC2034
# shellcheck disable=SC2329
(
	fEntryPoint(){
		local -i count_Tests=0
		local -i count_Passed=0
		local -i count_Failed=0
	:;}
	fRunTest(){
		local -r  testMode="${1:-}"   ; shift || true   ## 'equal', 'notequal', 'error'.
		local -r  expectVal="${1:-}"  ; shift || true   ## Inherit from parent instead.
		local -r  cmdStr="${1:-}"     ; shift || true
	:;}
	fRunChained_TestLast(){
		local -r  testMode="${1:-}"   ; shift || true   ## 'equal', 'notequal', 'error'.
		local -r  expectVal="${1:-}"  ; shift || true   ## Inherit from parent instead.
		local -r  cmdStrs="${1:-}"    ; shift || true   ## >=1 commands with ';' as delimiter.
	:;}
	fPipe_LogAndShowPartialOutput_InitLogfile(){
		local filePath_Log="${1:-}" ; shift || true  ## If you want to override the logfile path. Otherwise it's the path of this script+basename, + '.log'.
	:;}
	fPipe_LogAndShowPartialOutput(){ :; }
	fPipe_LogOnly(){ :; }
	fGetIsolatedExeName(){
		local -n  retVarName_CmdName_1myq1b5="${1:-}"   ; shift || true   ## The parent variable to populate with the isolated command 'basename' (no path).
		local -n  retVarName_TheRest_1myq1b5="${1:-}"   ; shift || true   ## The parent variable to populate with the rest of the command-line after the executable.
		local -r  commandString="${1:-}"                ; shift || true   ## The full command line
	:;}
	fScrambleString(){
		local -n  outputVarName_1myn9vt=${1:-}   ; shift || true  ## The parent variable to put the results in. The results should have no spaces, unless a space is one of the inputs as a symbol to randomize. But will still work with spaces.
		local -r  inputSymbolList="${1:-}"       ; shift || true  ## List of symbols to scramble, as a regular UTF-8 bash string. Will have no spaces or delimiters, unless a space is one of the inputs as a symbol to randomize.
		local -ri outputLen=${1:-1}              ; shift || true  ## Output scrambled string length
		local -ri canRepeatChars=${1:-1}         ; shift || true  ## 0: Don't repeat any symbols if possible (i.e. if input len > output len). 1: Try to repeat symbols in the random output.
	}
	fTallyResult(){
		local -ri errNum=${1:-0}      ; shift || true  ## The integer return value from the command.
		local -r  testMode="${1:-}"   ; shift || true  ## 'equal', 'notequal', 'error'.
		local -r  expectVal="${1:-}"  ; shift || true  ##
		local -r  gotVal="${1:-}"     ; shift || true  ##
	:;}
	fEcho_ResetBlankCounter()     { :; }
	fEcho_WasLastEchoBlank_Set()  { local -i arg1=${1:-0}; }
	fEcho_WasLastEchoBlank_Get()  { return 0; }
	fEcho_IsInRawInlineMode_Set() { local -i arg1=${1:-0}; }
	fEcho_IsInRawInlineMode_Get() { return 0; }
	fEcho_Clean()                 { local arg1="${1:-0}"; }
	fEcho()                       { local arg1="${1:-0}"; }
	fEcho_Force()                 { local arg1="${1:-0}"; }
	fEcho_Clean_Force()           { local arg1="${1:-0}"; }
)

#••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••
## Generic function(s) that can't be 'sourced'.
#••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••
fResolvePath(){
	##	Purpose:
	##		- Resolves an argument to a canonical full path, while being careful to not be too broad as to resolve to something else with the same name.
	##		- Resolution priority:
	##			- Exactly as specified.
	##			- "[this script's path]/lib/[specified name if given without a path]"
	##			- "[this script's path]/include/[specified name if given without a path]"
	##			- "[this script's path]/includes/[specified name if given without a path]"
	##			- If specified a name without a path: Find in $PATH
	##			- If doesn't have to exist, and still haven't found it, then just canonicalize original argument
	local -n parentVarName_ResolvedPath_t4rej=${1:-}  ; shift || true  ## Parent variable to store fully resolved path in.
	local    nameOrPath="${1:-}"                      ; shift || true  ## File or folder path (relative or absolute). If an executable file, can be just a name to search in $PATH, to fully resolve.
	local -i mustExist=${1:-0}                        ; shift || true  ## 1 [default]: path must exist or error occurs. 0: Just rationalize paths, doesn't have to exist.
	parentVarName_ResolvedPath_t4rej=""
	[[   -z "${nameOrPath}" ]]  &&  { echo -e "\nError in $(basename "${BASH_SOURCE[0]}")·${FUNCNAME[0]}(): No file or directory specified to resolve.\n"; fEcho_WasLastEchoBlank_Set 1; return 1; }
	local -r mePath_t4rmy="$(dirname "${BASH_SOURCE[0]}")"
	local -i isExeWithNoPath=0 ; [[ "${nameOrPath}" == "$(basename "${nameOrPath}")" ]] && isExeWithNoPath=1 ; readonly isExeWithNoPath
	local    testPath="${nameOrPath}"
	{ [[ ! -e "${testPath}"   ]]                          ; }  &&  testPath="${mePath_t4rmy}/${nameOrPath}"
	{ [[ ! -e "${testPath}"   ]] && ((isExeWithNoPath))   ; }  &&  testPath="${mePath_t4rmy}/lib/${nameOrPath}"
	{ [[ ! -e "${testPath}"   ]] && ((isExeWithNoPath))   ; }  &&  testPath="${mePath_t4rmy}/include/${nameOrPath}"
	{ [[ ! -e "${testPath}"   ]] && ((isExeWithNoPath))   ; }  &&  testPath="${mePath_t4rmy}/includes/${nameOrPath}"
	{ [[ ! -e "${testPath}"   ]] && ((isExeWithNoPath))   ; }  &&  testPath="$(which "${nameOrPath}" 2>/dev/null || true)"
	{ [[ ! -e "${testPath}"   ]] && ((mustExist))         ; }  &&  { echo -e "\nError in $(basename "${BASH_SOURCE[0]}")·${FUNCNAME[0]}(): Could not resolve path '${nameOrPath}' [£ǝŔc].\n"; fEcho_WasLastEchoBlank_Set 1; return 1; }
	{ [[ ! -e "${testPath}"   ]] || [[ -z "${testPath}" ]]; }  &&  testPath="${nameOrPath}"  ## Revert to original definition
	if ((mustExist)); then testPath="$(realpath -e "${testPath}" 2>/dev/null || true)"
	else                   testPath="$(realpath -m "${testPath}" 2>/dev/null || true)"; fi
	## Last check to fail on
	{ [[ -z "${testPath}" ]] || { [[ ! -e "${testPath}" ]] && ((mustExist)); }; }  &&  { echo -e "\nError in $(basename "${BASH_SOURCE[0]}")·${FUNCNAME[0]}(): Could not resolve path '${nameOrPath}' [£ǝŔs].\n"; fEcho_WasLastEchoBlank_Set 1; return 1; }
	# shellcheck disable=SC2034
	parentVarName_ResolvedPath_t4rej="${testPath}"
}


#••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••
# Entry point
#••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••

if [[ -z "${meName_t4rgd+x}" ]]; then
	declare -r mePath_t4rgd="$(realpath -e "${BASH_SOURCE[0]}")"
	declare -r meName_t4rgd="$(basename "${mePath_t4rgd}")"
	declare -r meDir_t4rgd="$(dirname "${mePath_t4rgd}")"
	declare -r serialDT_t4rgd="$(date "+%Y%m%d-%H%M%S")"
fi


## Make sure relative paths work
cd "${meDir_t4rgd}"

## Source the generic script 'utility/n8lib_test'. It will call fMain() above.
declare n8test_resolved="utility/include/n8lib_test"
fResolvePath  n8test_resolved  "${n8test_resolved}" ; readonly n8test_resolved
#echo "n8test_resolved: '${n8test_resolved}'"; exit
source "${n8test_resolved}"

## Initialize logging (fPipe_LogAndShowPartialOutput_InitLogfile() is defined in 'n8lib_test')
declare logFile="${mePath_t4rgd%.*}.log"
fResolvePath  logFile    "${logFile}"  0
fPipe_LogAndShowPartialOutput_InitLogfile "${logFile}"

## Kick off testing (functions are defined in 'n8lib_test')
fEntryPoint | fPipe_LogAndShowPartialOutput


#••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••
##	Script history:
#••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••
##		- 20260420 JC: Copied test.sh to test_against_v2.sh.
##		- 20260425 JC: Finished.
##		- 20260427 JC:
##			- Updated fResolvePath().
##			- Fixed bugs in loops natural end, caused by not setting `set +e`.
##		- 20260428 JC: Removed now-unnecessary reference to alias-definitions.sh.
##		- 20260511 JC: Renamed to *.bash to make it clear it's not a POSIX shell.
##		- 20260519 JC:
##			- Updated for updated n8lib_test.
##			- Changed license from GPL2 to MIT.
