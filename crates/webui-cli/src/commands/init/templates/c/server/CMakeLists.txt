cmake_minimum_required(VERSION 3.24)
project(__APP_NAME__ LANGUAGES C)
set(CMAKE_C_STANDARD 11)
set(CMAKE_C_STANDARD_REQUIRED ON)
include(${CMAKE_CURRENT_LIST_DIR}/native.cmake)
add_executable(webui-app main.c)
target_link_libraries(webui-app PRIVATE webui_sdk civetweb)
set_target_properties(webui-app PROPERTIES
  BUILD_RPATH "${WEBUI_SDK_DIR}/lib"
  INSTALL_RPATH "${WEBUI_SDK_DIR}/lib")
if(WIN32)
  add_custom_command(TARGET webui-app POST_BUILD COMMAND ${CMAKE_COMMAND} -E
    copy_if_different "${WEBUI_SDK_DIR}/lib/webui_ffi.dll" "$<TARGET_FILE_DIR:webui-app>")
endif()
