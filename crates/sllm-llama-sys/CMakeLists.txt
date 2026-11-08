cmake_minimum_required(VERSION 3.20)
project(sllm_llama_sys LANGUAGES C CXX)

set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_CXX_EXTENSIONS OFF)
set(CMAKE_POSITION_INDEPENDENT_CODE ON)

if(NOT SLLM_LLAMA_SRC)
    message(FATAL_ERROR "SLLM_LLAMA_SRC is not set")
endif()

if(NOT EXISTS "${SLLM_LLAMA_SRC}/include/llama.h")
    message(FATAL_ERROR "SLLM_LLAMA_SRC does not look like llama.cpp: ${SLLM_LLAMA_SRC}")
endif()

# These defaults are set in the cache so they win over llama.cpp's standalone
# defaults when it is added as a subdirectory.
set(LLAMA_BUILD_COMMON ON CACHE BOOL "" FORCE)
set(LLAMA_BUILD_MTMD ON CACHE BOOL "" FORCE)
set(LLAMA_BUILD_TOOLS OFF CACHE BOOL "" FORCE)
set(LLAMA_BUILD_TESTS OFF CACHE BOOL "" FORCE)
set(LLAMA_BUILD_EXAMPLES OFF CACHE BOOL "" FORCE)
set(LLAMA_BUILD_SERVER OFF CACHE BOOL "" FORCE)
set(LLAMA_BUILD_APP OFF CACHE BOOL "" FORCE)
set(LLAMA_BUILD_UI OFF CACHE BOOL "" FORCE)
set(LLAMA_SUBPROCESS OFF CACHE BOOL "" FORCE)

add_subdirectory("${SLLM_LLAMA_SRC}" "${CMAKE_CURRENT_BINARY_DIR}/llama")

add_library(sllm_ffi STATIC "${CMAKE_CURRENT_SOURCE_DIR}/src/sllm_shim.cpp")
add_library(sllm::ffi ALIAS sllm_ffi)

target_include_directories(sllm_ffi
    PUBLIC
        "${SLLM_LLAMA_SRC}/include"
        "${SLLM_LLAMA_SRC}/ggml/include"
    PRIVATE
        "${CMAKE_CURRENT_SOURCE_DIR}/include"
)

target_compile_features(sllm_ffi PRIVATE cxx_std_17)

if(MSVC)
    target_compile_options(sllm_ffi PRIVATE /EHsc /GR)
else()
    target_compile_options(sllm_ffi PRIVATE -fexceptions -frtti -fvisibility=hidden)
endif()

target_link_libraries(sllm_ffi PUBLIC mtmd llama-common llama ggml)

if(ANDROID)
    # 16 KB page-size devices need at least this alignment for the final .so.
    target_link_options(sllm_ffi PUBLIC "-Wl,-z,max-page-size=16384")
endif()

install(TARGETS sllm_ffi
    ARCHIVE DESTINATION lib
    LIBRARY DESTINATION lib
    RUNTIME DESTINATION bin
)

# llama.cpp does not install these private static dependencies, but a Rust link
# step needs them because it does not see CMake's transitive link graph.
install(TARGETS llama-common-base vendor-hash
    ARCHIVE DESTINATION lib
    LIBRARY DESTINATION lib
    RUNTIME DESTINATION bin
)
